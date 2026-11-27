//! MCP sidecar for Keepwright.
//!
//! Offline mode links `keep-core` directly (architecture section 7.2): no game
//! process, no GPU and no async runtime. Live mode forwards tool calls to a
//! running game over a loopback HTTP/1.1 JSON-RPC connection ([`LiveClient`]).
//! Either way it speaks line-delimited JSON-RPC 2.0 over stdio, which makes it
//! deterministic, cheap to spawn and trivial to replay in tests.
//!
//! The protocol handler is a plain library type ([`McpServer`]) so every tool,
//! error case and atomic rollback can be unit tested without a subprocess. The
//! `keep-mcp` binary is a small stdin/stdout loop around it.

use keep_core::command::AuthorId;
use keep_core::document::CastleDocument;
use keep_core::plan_render::{render_plan_svg, PlanView};
use keep_core::proposal::AgentPermission;
use keep_core::session::Session;
use keep_core::style::StylePack;
use keep_core::terrain::SiteTerrain;
use keep_core::validate::validate_structure;
use keep_proto::{
    ApplyOperationsRequest, ApplyOperationsResponse, AssessmentResponse, BeginProposalRequest,
    ElementListResponse, GetCastleResponse, GetHistoryResponse, GetSiteResponse, GetStyleResponse,
    HistoryEntry, ListProposalsResponse, ListStylesResponse, ProposalResponse, RenderPlanRequest,
    RenderPlanResponse, RenderViewRequest, RenderViewResponse, StageProposalOpsRequest,
    SubmitProposalRequest, UndoResponse,
};
use serde::Serialize;
use serde_json::{json, Value};

mod live;

pub use live::LiveClient;

/// MCP protocol revision implemented by this sidecar.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Server name reported during `initialize`.
pub const SERVER_NAME: &str = "keep-mcp";
/// Author identity written into the command log for offline agent edits.
pub const OFFLINE_AUTHOR_ID: &str = "mcp-offline";

/// JSON-RPC error codes used by the handler.
mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    pub const RESOURCE_NOT_FOUND: i64 = -32002;
}

/// Which authoritative backend a request is served from.
#[derive(Debug)]
enum Backend {
    /// `keep-core` linked in-process: no game process, no GPU.
    Offline,
    /// Forward every tool call to the running game's loopback control server.
    Live(LiveClient),
}

/// The MCP server: one authoritative [`Session`] plus the agent author identity
/// used for edits and undos. In live mode the session stays empty and tool calls
/// are forwarded to the game, which owns the real document.
#[derive(Debug)]
pub struct McpServer {
    session: Session,
    author: AuthorId,
    permission: AgentPermission,
    backend: Backend,
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new(false)
    }
}

impl McpServer {
    /// Create a server with direct-edit permission. `empty` starts from an empty
    /// document instead of the M0b showcase.
    pub fn new(empty: bool) -> Self {
        let document = if empty {
            CastleDocument::new()
        } else {
            keep_core::fixtures::m0b_showcase_document()
        };
        Self::with_permission(
            keep_core::session::DEFAULT_SITE_ID,
            SiteTerrain::m0a_promontory(),
            document,
            AgentPermission::DirectEdit,
        )
    }

    /// Create a server around an explicit site terrain, document and permission.
    pub fn with_permission(
        site_id: impl Into<String>,
        terrain: SiteTerrain,
        document: CastleDocument,
        permission: AgentPermission,
    ) -> Self {
        let site_id = site_id.into();
        let mut session = Session::from_document(site_id, terrain, document);
        session
            .register_agent(OFFLINE_AUTHOR_ID, "Offline agent", "#4fa3ff", permission)
            .expect("offline agent identity is valid");
        Self {
            session,
            author: AuthorId::agent(OFFLINE_AUTHOR_ID).expect("offline author id is valid"),
            permission,
            backend: Backend::Offline,
        }
    }

    /// Create a server around an explicit site terrain and document with the
    /// historical direct-edit default.
    pub fn from_document(
        site_id: impl Into<String>,
        terrain: SiteTerrain,
        document: CastleDocument,
    ) -> Self {
        Self::with_permission(site_id, terrain, document, AgentPermission::DirectEdit)
    }

    /// Create a server on a curated site, or `None` when the site id is unknown.
    pub fn with_start(site_id: &str, document: CastleDocument) -> Option<Self> {
        Self::with_start_permission(site_id, document, AgentPermission::DirectEdit)
    }

    /// Create a server on a curated site with an explicit agent permission.
    pub fn with_start_permission(
        site_id: &str,
        document: CastleDocument,
        permission: AgentPermission,
    ) -> Option<Self> {
        SiteTerrain::for_site(site_id)
            .map(|terrain| Self::with_permission(site_id, terrain, document, permission))
    }

    /// Create a live-mode server that forwards tool calls to a running game.
    /// The URL must be a loopback `http://127.0.0.1:<port>` or
    /// `http://localhost:<port>`.
    pub fn live(url: &str, token: &str, permission: AgentPermission) -> Result<Self, String> {
        let client = LiveClient::connect(url, token)?;
        let mut session = Session::from_document(
            keep_core::session::DEFAULT_SITE_ID,
            SiteTerrain::m0a_promontory(),
            CastleDocument::new(),
        );
        session
            .register_agent(OFFLINE_AUTHOR_ID, "Offline agent", "#4fa3ff", permission)
            .expect("offline agent identity is valid");
        Ok(Self {
            session,
            author: AuthorId::agent(OFFLINE_AUTHOR_ID).expect("offline author id is valid"),
            permission,
            backend: Backend::Live(client),
        })
    }

    /// The agent permission enforced for tool calls.
    pub fn permission(&self) -> AgentPermission {
        self.permission
    }

    /// True when this server forwards tool calls to a running game.
    pub fn is_live(&self) -> bool {
        matches!(self.backend, Backend::Live(_))
    }

    /// Current castle document (tests and embedders).
    pub fn document(&self) -> &CastleDocument {
        self.session.document()
    }

    /// The authoritative session (tests and embedders).
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Handle one line of stdin. Returns the JSON-RPC response line, or `None`
    /// for notifications and blank lines.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let message: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(error) => {
                return Some(
                    serde_json::to_string(&error_response(
                        Value::Null,
                        code::PARSE_ERROR,
                        format!("parse error: {error}"),
                    ))
                    .expect("error response serialises"),
                )
            }
        };
        self.handle_message(message)
            .map(|response| serde_json::to_string(&response).expect("response serialises"))
    }

    /// Handle one decoded JSON-RPC message.
    pub fn handle_message(&mut self, message: Value) -> Option<Value> {
        // Notifications (messages without an `id`) never receive a response,
        // even when the rest of the envelope is malformed.
        let id = message.get("id")?.clone();

        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(error_response(
                id,
                code::INVALID_REQUEST,
                "expected jsonrpc: \"2.0\"",
            ));
        }
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Some(error_response(
                id,
                code::INVALID_REQUEST,
                "missing method field",
            ));
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        let result = match method {
            "initialize" => Ok(initialize_result()),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": keep_proto::tool_definitions() })),
            "tools/call" => return Some(self.handle_tool_call(id, &params)),
            "resources/list" => Ok(resources_list()),
            "resources/read" => self.handle_resource_read(&params),
            _ => {
                return Some(error_response(
                    id,
                    code::METHOD_NOT_FOUND,
                    format!("unknown method: {method}"),
                ))
            }
        };

        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((error_code, message)) => error_response(id, error_code, message),
        })
    }

    fn handle_tool_call(&mut self, id: Value, params: &Value) -> Value {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return error_response(id, code::INVALID_PARAMS, "missing tool name");
        };
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        if let Some(message) = self.permission_refusal(name) {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": tool_error(message)
            });
        }

        if self.is_live() {
            return self.handle_live_tool_call(id, name, arguments);
        }

        let result = match name {
            "get_castle" => self.tool_get_castle().map(|value| tool_result(&value)),
            "list_elements" => self.tool_list_elements().map(|value| tool_result(&value)),
            "render_plan" => self
                .tool_render_plan(arguments)
                .map(|value| tool_result(&value)),
            "apply_operations" => self
                .tool_apply_operations(arguments)
                .map(|value| tool_result(&value)),
            "undo" => self.tool_undo().map(|value| tool_result(&value)),
            "get_history" => self.tool_get_history().map(|value| tool_result(&value)),
            "get_assessment" => self.tool_get_assessment().map(|value| tool_result(&value)),
            "list_styles" => self.tool_list_styles().map(|value| tool_result(&value)),
            "get_style" => self
                .tool_get_style(arguments)
                .map(|value| tool_result(&value)),
            "get_site" => self.tool_get_site().map(|value| tool_result(&value)),
            "begin_proposal" => self
                .tool_begin_proposal(arguments)
                .map(|value| tool_result(&value)),
            "stage_proposal_ops" => self
                .tool_stage_proposal_ops(arguments)
                .map(|value| tool_result(&value)),
            "submit_proposal" => self
                .tool_submit_proposal(arguments)
                .map(|value| tool_result(&value)),
            "list_proposals" => self.tool_list_proposals().map(|value| tool_result(&value)),
            "render_view" => Err(
                "render_view requires --live mode with a running game; use render_plan offline"
                    .to_string(),
            ),
            other => {
                return error_response(id, code::INVALID_PARAMS, format!("unknown tool: {other}"))
            }
        };

        match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(message) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": tool_error(message)
            }),
        }
    }

    /// Refuse tools the connected agent's permission forbids (architecture
    /// section 7.6). Reads and renders are always allowed.
    fn permission_refusal(&self, tool: &str) -> Option<String> {
        match self.permission {
            AgentPermission::DirectEdit => None,
            AgentPermission::ReadOnly => matches!(
                tool,
                "apply_operations"
                    | "undo"
                    | "begin_proposal"
                    | "stage_proposal_ops"
                    | "submit_proposal"
            )
            .then(|| {
                format!(
                    "permission denied: read-only agents cannot call '{tool}'. \
                     Ask the human to raise this agent's permission to proposal-only or direct-edit."
                )
            }),
            AgentPermission::ProposalOnly => match tool {
                "apply_operations" | "undo" => Some(format!(
                    "permission denied: proposal-only agents cannot call '{tool}' directly. \
                     Use begin_proposal, stage_proposal_ops and submit_proposal instead; \
                     the human accepts or rejects the proposal in-game."
                )),
                _ => None,
            },
        }
    }

    fn handle_live_tool_call(&mut self, id: Value, name: &str, arguments: Value) -> Value {
        let result = self.dispatch_live_tool(name, arguments);
        match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(message) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": tool_error(message)
            }),
        }
    }

    fn dispatch_live_tool(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        // Never forward an agent-supplied identity: the game decides who the
        // caller is from the control token, not from tool arguments (M1.3 B2).
        let arguments = strip_agent_id(arguments);

        match name {
            "get_castle" => {
                let value = self.call_live(name, arguments)?;
                let response: GetCastleResponse = serde_json::from_value(value)
                    .map_err(|error| format!("invalid get_castle response: {error}"))?;
                Ok(tool_result(&response))
            }
            "list_elements" => {
                let value = self.call_live(name, arguments)?;
                let response: ElementListResponse = serde_json::from_value(value)
                    .map_err(|error| format!("invalid list_elements response: {error}"))?;
                Ok(tool_result(&response))
            }
            "render_view" => {
                let request: RenderViewRequest = serde_json::from_value(arguments)
                    .map_err(|error| format!("invalid arguments: {error}"))?;
                let params = serde_json::to_value(&request)
                    .map_err(|error| format!("invalid arguments: {error}"))?;
                let value = self.call_live(name, params)?;
                let response: RenderViewResponse = serde_json::from_value(value)
                    .map_err(|error| format!("invalid render_view response: {error}"))?;
                Ok(image_tool_result(&response))
            }
            _ => self
                .call_live(name, arguments)
                .map(|value| tool_result(&value)),
        }
    }

    /// Forward one raw request to the live game and return its `result`.
    fn call_live(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let Backend::Live(client) = &mut self.backend else {
            return Err("internal error: live backend is not connected".to_string());
        };
        client.call(method, params)
    }

    fn tool_get_castle(&self) -> Result<GetCastleResponse, String> {
        Ok(keep_proto::build_castle_response(&self.session))
    }

    fn tool_list_elements(&self) -> Result<ElementListResponse, String> {
        Ok(keep_proto::build_element_list_response(&self.session))
    }

    fn tool_render_plan(&self, arguments: Value) -> Result<RenderPlanResponse, String> {
        let request: RenderPlanRequest = serde_json::from_value(arguments)
            .map_err(|error| format!("invalid arguments: {error}"))?;
        let (view_name, view) = match request.view.as_str() {
            "top_plan" => ("top_plan", PlanView::TopPlan),
            "elevation_north" => ("elevation_north", PlanView::ElevationNorth),
            other => {
                return Err(format!(
                    "invalid view '{other}': expected 'top_plan' or 'elevation_north'"
                ))
            }
        };
        let svg = render_plan_svg(self.session.document(), self.session.terrain(), view);
        Ok(RenderPlanResponse {
            view: view_name.to_string(),
            document_hash: hex_hash(self.session.document().document_hash()),
            mime_type: "image/svg+xml".to_string(),
            svg,
        })
    }

    /// Apply a batch atomically through the shared session engine. Validation,
    /// id allocation, inverse records and logging all live in `keep-core`, so
    /// the offline sidecar enforces exactly the same rules as the game host.
    fn tool_apply_operations(
        &mut self,
        arguments: Value,
    ) -> Result<ApplyOperationsResponse, String> {
        let request: ApplyOperationsRequest = serde_json::from_value(arguments)
            .map_err(|error| format!("invalid arguments: {error}"))?;
        let applied_count = request.operations.len();
        let ops = request.operations;

        let outcome = self
            .session
            .apply_batch(self.author.clone(), ops)
            .map_err(|error| error.to_string())?;
        let scene = self.session.build_scene();

        Ok(ApplyOperationsResponse {
            applied_count,
            created_ids: outcome.created_ids,
            document_hash: hex_hash(outcome.document_hash),
            layout_hash: hex_hash(scene.layout_hash),
            block_count: scene.block_count,
            chunk_count: scene.chunk_count(),
            summary: format!(
                "Applied {applied_count} operation(s): {}; {} block(s) in {} chunk(s)",
                outcome.summary,
                scene.block_count,
                scene.chunk_count()
            ),
        })
    }

    /// Compensate this agent's latest still-live batch.
    fn tool_undo(&mut self) -> Result<UndoResponse, String> {
        let outcome = self
            .session
            .undo_last(&self.author)
            .map_err(|error| error.to_string())?;
        let scene = self.session.build_scene();
        Ok(UndoResponse {
            seq: outcome.undo_seq,
            compensated_seq: outcome.target_seq,
            document_hash: hex_hash(outcome.document_hash),
            layout_hash: hex_hash(scene.layout_hash),
            block_count: scene.block_count,
            chunk_count: scene.chunk_count(),
            summary: outcome.summary,
        })
    }

    /// Return the append-only command log with undone markers.
    fn tool_get_history(&self) -> Result<GetHistoryResponse, String> {
        let entries: Vec<HistoryEntry> = self
            .session
            .history()
            .iter()
            .map(|command| HistoryEntry {
                seq: command.seq,
                command_id: command.command_id.clone(),
                author: command.author.to_string(),
                summary: command.summary.clone(),
                touched_elements: command.touched_elements.clone(),
                undone: self.session.is_undone(command.seq),
            })
            .collect();
        Ok(GetHistoryResponse {
            count: entries.len(),
            entries,
        })
    }

    /// Assess the castle against the Edwardian Welsh rules and cite the style.
    fn tool_get_assessment(&self) -> Result<AssessmentResponse, String> {
        let style = StylePack::edwardian_welsh();
        let report = validate_structure(self.session.document(), self.session.terrain(), &style);
        Ok(AssessmentResponse::new(style.id.clone(), report))
    }

    /// Every curated style pack, in stable id order.
    fn tool_list_styles(&self) -> Result<ListStylesResponse, String> {
        Ok(ListStylesResponse {
            styles: StylePack::list(),
        })
    }

    /// One style pack by id.
    fn tool_get_style(&self, arguments: Value) -> Result<GetStyleResponse, String> {
        let id = arguments
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "invalid arguments: missing string field 'id'".to_string())?;
        let style = StylePack::find(id)
            .ok_or_else(|| format!("unknown style id '{}'", truncate_message_field(id, 64)))?;
        Ok(GetStyleResponse { style })
    }

    /// The active curated site plus the full site catalogue.
    fn tool_get_site(&self) -> Result<GetSiteResponse, String> {
        let available_sites = SiteTerrain::list_sites();
        let site = available_sites
            .iter()
            .find(|site| site.id == self.session.site_id())
            .cloned()
            .ok_or_else(|| format!("unknown active site '{}'", self.session.site_id()))?;
        Ok(GetSiteResponse {
            site,
            available_sites,
        })
    }

    /// Begin a draft proposal overlay owned by this agent.
    fn tool_begin_proposal(&mut self, arguments: Value) -> Result<ProposalResponse, String> {
        let request: BeginProposalRequest = serde_json::from_value(arguments)
            .map_err(|error| format!("invalid arguments: {error}"))?;
        let proposal = self
            .session
            .begin_proposal(self.author.clone(), &request.title)
            .map_err(|error| error.to_string())?;
        Ok(ProposalResponse { proposal })
    }

    /// Stage an additional batch onto a draft proposal without touching the
    /// live document.
    fn tool_stage_proposal_ops(&mut self, arguments: Value) -> Result<ProposalResponse, String> {
        let request: StageProposalOpsRequest = serde_json::from_value(arguments)
            .map_err(|error| format!("invalid arguments: {error}"))?;
        let proposal = self
            .session
            .stage_proposal_ops(&request.proposal_id, &self.author, request.operations)
            .map_err(|error| error.to_string())?;
        Ok(ProposalResponse { proposal })
    }

    /// Submit a non-empty draft for human approval.
    fn tool_submit_proposal(&mut self, arguments: Value) -> Result<ProposalResponse, String> {
        let request: SubmitProposalRequest = serde_json::from_value(arguments)
            .map_err(|error| format!("invalid arguments: {error}"))?;
        let proposal = self
            .session
            .submit_proposal(&request.proposal_id, &self.author, &request.summary)
            .map_err(|error| error.to_string())?;
        Ok(ProposalResponse { proposal })
    }

    /// Every proposal overlay in stable id order.
    fn tool_list_proposals(&self) -> Result<ListProposalsResponse, String> {
        let proposals: Vec<_> = self.session.proposals().cloned().collect();
        Ok(ListProposalsResponse {
            count: proposals.len(),
            proposals,
        })
    }

    fn handle_resource_read(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let Some(uri) = params.get("uri").and_then(Value::as_str) else {
            return Err((code::INVALID_PARAMS, "missing resource uri".into()));
        };
        if self.is_live() {
            return self.handle_live_resource_read(uri);
        }
        match uri {
            "keep://castle/document" => {
                let castle = self
                    .tool_get_castle()
                    .map_err(|message| (code::INVALID_PARAMS, message))?;
                let text = serde_json::to_string_pretty(&castle)
                    .map_err(|error| (code::INVALID_PARAMS, error.to_string()))?;
                Ok(resource_contents(uri, "application/json", text))
            }
            "keep://castle/save" => {
                let text = self
                    .session
                    .to_save_json()
                    .map_err(|error| (code::INTERNAL_ERROR, error.to_string()))?;
                Ok(resource_contents(uri, "application/json", text))
            }
            "keep://castle/plan/top" => Ok(self.resource_svg(uri, PlanView::TopPlan)),
            "keep://castle/plan/elevation" => Ok(self.resource_svg(uri, PlanView::ElevationNorth)),
            "keep://castle/assessment" => {
                let assessment = self
                    .tool_get_assessment()
                    .map_err(|message| (code::INTERNAL_ERROR, message))?;
                let text = serde_json::to_string_pretty(&assessment)
                    .map_err(|error| (code::INTERNAL_ERROR, error.to_string()))?;
                Ok(resource_contents(uri, "application/json", text))
            }
            "keep://styles/edwardian_welsh" => {
                let style = StylePack::edwardian_welsh();
                let text = serde_json::to_string_pretty(&style)
                    .map_err(|error| (code::INTERNAL_ERROR, error.to_string()))?;
                Ok(resource_contents(uri, "application/json", text))
            }
            other => Err((
                code::RESOURCE_NOT_FOUND,
                format!("unknown resource: {other}"),
            )),
        }
    }

    /// Serve a resource from the running game (M1.3 H4). The offline dummy
    /// session is empty, so every read must be forwarded to the live control
    /// server instead of being answered locally.
    fn handle_live_resource_read(&mut self, uri: &str) -> Result<Value, (i64, String)> {
        let (method, params, mime_type) = match uri {
            "keep://castle/document" => ("get_castle", json!({}), "application/json"),
            "keep://castle/assessment" => ("get_assessment", json!({}), "application/json"),
            "keep://castle/plan/top" => (
                "render_plan",
                json!({ "view": "top_plan" }),
                "image/svg+xml",
            ),
            "keep://castle/plan/elevation" => (
                "render_plan",
                json!({ "view": "elevation_north" }),
                "image/svg+xml",
            ),
            "keep://styles/edwardian_welsh" => (
                "get_style",
                json!({ "id": "edwardian_welsh" }),
                "application/json",
            ),
            "keep://castle/save" => {
                return Err((
                    code::INVALID_PARAMS,
                    "keep://castle/save is only available in --offline mode".to_string(),
                ))
            }
            other => {
                return Err((
                    code::RESOURCE_NOT_FOUND,
                    format!("unknown resource: {other}"),
                ))
            }
        };

        let value = self
            .call_live(method, params)
            .map_err(|message| (code::INTERNAL_ERROR, message))?;
        let text = if mime_type == "image/svg+xml" {
            value
                .get("svg")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    (
                        code::INTERNAL_ERROR,
                        "live render_plan response had no svg field".to_string(),
                    )
                })?
        } else {
            serde_json::to_string_pretty(&value)
                .map_err(|error| (code::INTERNAL_ERROR, error.to_string()))?
        };
        Ok(resource_contents(uri, mime_type, text))
    }

    fn resource_svg(&self, uri: &str, view: PlanView) -> Value {
        let svg = render_plan_svg(self.session.document(), self.session.terrain(), view);
        resource_contents(uri, "image/svg+xml", svg)
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {}, "resources": {} },
        "serverInfo": {
            "name": SERVER_NAME,
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "Keepwright castle sidecar. Use get_castle, list_elements, render_plan, apply_operations, undo and get_history, plus get_assessment, list_styles, get_style and get_site for structural and style reads. Use begin_proposal, stage_proposal_ops and submit_proposal to propose edits for the human to accept in-game (list_proposals shows their status). render_view needs live mode; render_plan works offline."
    })
}

fn resources_list() -> Value {
    json!({
        "resources": [
            {
                "uri": "keep://castle/document",
                "name": "Castle document",
                "description": "Semantic castle summary with elements, openings and hashes.",
                "mimeType": "application/json"
            },
            {
                "uri": "keep://castle/save",
                "name": "Castle save",
                "description": "Versioned castle.keep.json save file with the initial document and command log.",
                "mimeType": "application/json"
            },
            {
                "uri": "keep://castle/plan/top",
                "name": "Top plan",
                "description": "Deterministic SVG architectural plan over terrain contours.",
                "mimeType": "image/svg+xml"
            },
            {
                "uri": "keep://castle/plan/elevation",
                "name": "North elevation",
                "description": "Deterministic SVG north elevation with the terrain profile.",
                "mimeType": "image/svg+xml"
            },
            {
                "uri": "keep://castle/assessment",
                "name": "Structural assessment",
                "description": "Edwardian Welsh structural findings with severity, source notes and suggested fixes.",
                "mimeType": "application/json"
            },
            {
                "uri": "keep://styles/edwardian_welsh",
                "name": "Edwardian Welsh style pack",
                "description": "Curated dimension, slenderness, batter, plinth and flanking rules with source notes.",
                "mimeType": "application/json"
            }
        ]
    })
}

fn resource_contents(uri: &str, mime_type: &str, text: String) -> Value {
    json!({
        "contents": [
            {
                "uri": uri,
                "mimeType": mime_type,
                "text": text
            }
        ]
    })
}

fn tool_result<T: Serialize>(value: &T) -> Value {
    let structured = serde_json::to_value(value).expect("tool result serialises");
    let text = serde_json::to_string_pretty(&structured).expect("tool result serialises");
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": structured,
        "isError": false
    })
}

fn tool_error(message: impl Into<String>) -> Value {
    json!({
        "content": [{ "type": "text", "text": message.into() }],
        "isError": true
    })
}

/// MCP image content plus the structured render metadata for `render_view`.
fn image_tool_result(response: &RenderViewResponse) -> Value {
    let structured = serde_json::to_value(response).expect("render_view result serialises");
    json!({
        "content": [{
            "type": "image",
            "data": response.image_base64,
            "mimeType": response.mime_type
        }],
        "structuredContent": structured,
        "isError": false
    })
}

fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() }
    })
}

fn hex_hash(hash: u64) -> String {
    format!("{hash:016x}")
}

/// Clamp untrusted input before echoing it back in an error message: control
/// characters become spaces and the result is limited to `max_chars`.
fn truncate_message_field(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .take(max_chars)
        .collect()
}

/// Drop any agent-supplied `agent_id` from live tool arguments. The live game
/// authenticates the caller from the control token, so forwarding an identity
/// from the model would let it act as another agent (M1.3 B2).
fn strip_agent_id(mut arguments: Value) -> Value {
    if let Some(object) = arguments.as_object_mut() {
        object.remove("agent_id");
    }
    arguments
}

#[cfg(test)]
mod tests {
    use super::*;
    use keep_proto::{MAX_BATCH_OPS, MAX_NAME_CHARS};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    fn server() -> McpServer {
        McpServer::new(false)
    }

    fn m1_server() -> McpServer {
        McpServer::with_start(
            "coastal_headland",
            keep_core::fixtures::m1_edwardian_document(),
        )
        .expect("coastal_headland is a curated site")
    }

    fn call(server: &mut McpServer, id: u64, method: &str, params: Value) -> Value {
        let response = server
            .handle_message(json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params
            }))
            .expect("request has a response");
        assert_eq!(response["id"], id);
        response
    }

    fn call_tool(server: &mut McpServer, name: &str, arguments: Value) -> Value {
        call(
            server,
            99,
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )["result"]
            .clone()
    }

    fn structured(server: &mut McpServer, name: &str, arguments: Value) -> Value {
        let result = call_tool(server, name, arguments);
        assert_eq!(result["isError"], false, "tool failed: {result}");
        assert!(result["content"][0]["text"].is_string());
        result["structuredContent"].clone()
    }

    fn tool_error_message(server: &mut McpServer, name: &str, arguments: Value) -> String {
        let result = call_tool(server, name, arguments);
        assert_eq!(result["isError"], true, "expected error, got {result}");
        result["content"][0]["text"].as_str().unwrap().to_string()
    }

    fn sample_wall(ref_id: &str) -> Value {
        json!({
            "op": "create_wall_run",
            "ref_id": ref_id,
            "name": "Test curtain",
            "start_mm": [-10_000, 6_000],
            "end_mm": [10_000, 6_000],
            "height_mm": 8_000,
            "thickness_mm": 2_000
        })
    }

    #[test]
    fn initialize_reports_server_and_capabilities() {
        let mut server = server();
        let response = call(&mut server, 1, "initialize", json!({}));
        assert_eq!(response["result"]["serverInfo"]["name"], "keep-mcp");
        assert_eq!(
            response["result"]["protocolVersion"],
            PROTOCOL_VERSION.to_string()
        );
        assert!(response["result"]["capabilities"]["tools"].is_object());
        assert!(response["result"]["capabilities"]["resources"].is_object());
    }

    #[test]
    fn notifications_get_no_response() {
        let mut server = server();
        assert!(server
            .handle_message(json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }))
            .is_none());
        assert!(server.handle_line("").is_none());
        // Malformed notifications still get no response.
        assert!(server.handle_message(json!({ "method": "ping" })).is_none());
        assert!(server
            .handle_message(json!({ "jsonrpc": "1.0", "method": "tools/list" }))
            .is_none());
        assert!(server.handle_message(json!({ "jsonrpc": "2.0" })).is_none());
        assert!(server.handle_message(json!([])).is_none());
    }

    #[test]
    fn unknown_tool_is_an_invalid_params_error() {
        let mut server = server();
        let response = call(
            &mut server,
            31,
            "tools/call",
            json!({ "name": "no_such_tool", "arguments": {} }),
        );
        assert_eq!(response["error"]["code"], code::INVALID_PARAMS);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown tool"));
        assert!(response.get("result").is_none());
    }

    #[test]
    fn ping_and_unknown_method() {
        let mut server = server();
        let response = call(&mut server, 2, "ping", json!({}));
        assert_eq!(response["result"], json!({}));

        let response = call(&mut server, 3, "no/such/method", json!({}));
        assert_eq!(response["error"]["code"], code::METHOD_NOT_FOUND);
    }

    #[test]
    fn bad_json_and_bad_envelope_are_reported() {
        let mut server = server();
        let response: Value =
            serde_json::from_str(&server.handle_line("{not json").unwrap()).unwrap();
        assert_eq!(response["error"]["code"], code::PARSE_ERROR);

        let response = server
            .handle_message(json!({ "jsonrpc": "1.0", "id": 1, "method": "ping" }))
            .unwrap();
        assert_eq!(response["error"]["code"], code::INVALID_REQUEST);

        let response = server
            .handle_message(json!({ "jsonrpc": "2.0", "id": 1 }))
            .unwrap();
        assert_eq!(response["error"]["code"], code::INVALID_REQUEST);
    }

    #[test]
    fn tools_list_exposes_fifteen_tools_with_schemas() {
        let mut server = server();
        let response = call(&mut server, 4, "tools/list", json!({}));
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 15);
        for tool in tools {
            assert!(tool["inputSchema"].is_object());
            assert!(tool["outputSchema"].is_object());
            assert!(tool["annotations"].is_object());
        }
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            vec![
                "get_castle",
                "list_elements",
                "render_plan",
                "apply_operations",
                "undo",
                "get_history",
                "get_assessment",
                "list_styles",
                "get_style",
                "get_site",
                "begin_proposal",
                "stage_proposal_ops",
                "submit_proposal",
                "list_proposals",
                "render_view"
            ]
        );
    }

    #[test]
    fn get_castle_summarises_the_showcase() {
        let mut server = server();
        let castle = structured(&mut server, "get_castle", json!({}));
        assert_eq!(castle["element_count"], 3);
        assert_eq!(castle["opening_count"], 2);
        assert!(castle["block_count"].as_u64().unwrap() > 1_500);
        assert!(castle["chunk_count"].as_u64().unwrap() > 0);
        assert_eq!(castle["document_hash"].as_str().unwrap().len(), 16);
        assert_eq!(castle["layout_hash"].as_str().unwrap().len(), 16);
        assert_eq!(castle["elements"][0]["kind"], "wall");
        assert_eq!(castle["elements"][0]["name"], "North curtain");
        assert_eq!(castle["elements"][1]["kind"], "wall");
        assert_eq!(castle["elements"][2]["kind"], "tower");
        assert_eq!(castle["openings"][0]["kind"], "arrow_loop");
        assert_eq!(castle["openings"][0]["wall_id"], 1);
    }

    #[test]
    fn list_elements_matches_get_castle() {
        let mut server = server();
        let castle = structured(&mut server, "get_castle", json!({}));
        let list = structured(&mut server, "list_elements", json!({}));
        assert_eq!(list["element_count"], castle["element_count"]);
        assert_eq!(list["elements"], castle["elements"]);
        assert_eq!(list["openings"], castle["openings"]);
    }

    #[test]
    fn m1_element_summaries_match_the_output_schema() {
        let mut server = m1_server();
        let allowed = ["wall", "tower", "gatehouse", "ditch"];
        for tool in ["get_castle", "list_elements"] {
            let summary = structured(&mut server, tool, json!({}));
            let elements = summary["elements"].as_array().unwrap();
            assert!(!elements.is_empty(), "{tool} returned no elements");
            for element in elements {
                let kind = element["kind"].as_str().unwrap();
                assert!(allowed.contains(&kind), "unexpected kind {kind} in {tool}");
            }
        }

        let castle = structured(&mut server, "get_castle", json!({}));
        let elements = castle["elements"].as_array().unwrap();

        let towers: Vec<_> = elements.iter().filter(|e| e["kind"] == "tower").collect();
        assert_eq!(towers.len(), 3);
        for tower in &towers {
            assert!(tower["plan"].is_string(), "tower missing plan: {tower}");
            assert!(
                tower["orientation_deg"].is_i64(),
                "tower missing orientation: {tower}"
            );
        }
        let square = towers
            .iter()
            .find(|tower| tower["name"] == "Square tower")
            .unwrap();
        assert_eq!(square["plan"], "square");
        let d_shaped = towers
            .iter()
            .find(|tower| tower["name"] == "D-shaped tower")
            .unwrap();
        assert_eq!(d_shaped["plan"], "d_shaped");
        assert_eq!(d_shaped["orientation_deg"], 180);

        let gatehouse = elements
            .iter()
            .find(|element| element["kind"] == "gatehouse")
            .unwrap();
        assert_eq!(gatehouse["passage_width_mm"], 3_200);
        assert_eq!(gatehouse["passage_height_mm"], 4_200);
        assert_eq!(gatehouse["depth_mm"], 8_000);
        assert_ne!(
            gatehouse["thickness_mm"], 3_200,
            "gatehouse passage width must not overload thickness_mm"
        );

        let ditch = elements
            .iter()
            .find(|element| element["kind"] == "ditch")
            .unwrap();
        assert_eq!(ditch["depth_mm"], 2_600);
        assert_eq!(ditch["thickness_mm"], 6_000);
    }

    #[test]
    fn render_plan_returns_svg_for_both_views() {
        let mut server = server();
        for view in ["top_plan", "elevation_north"] {
            let plan = structured(&mut server, "render_plan", json!({ "view": view }));
            assert_eq!(plan["view"], view);
            assert_eq!(plan["mime_type"], "image/svg+xml");
            let svg = plan["svg"].as_str().unwrap();
            assert!(svg.starts_with("<svg "));
            assert!(svg.trim_end().ends_with("</svg>"));
        }
    }

    #[test]
    fn render_plan_with_empty_arguments_defaults_to_top_plan() {
        let mut server = server();
        let plan = structured(&mut server, "render_plan", json!({}));
        assert_eq!(plan["view"], "top_plan");
        assert_eq!(plan["mime_type"], "image/svg+xml");
        assert!(plan["svg"].as_str().unwrap().starts_with("<svg "));
    }

    #[test]
    fn invalid_view_is_a_tool_error() {
        let mut server = server();
        let before = structured(&mut server, "get_castle", json!({}));
        let message =
            tool_error_message(&mut server, "render_plan", json!({ "view": "isometric" }));
        assert!(message.contains("invalid view"));
        let after = structured(&mut server, "get_castle", json!({}));
        assert_eq!(before["document_hash"], after["document_hash"]);
    }

    #[test]
    fn apply_batch_can_reference_a_new_wall_within_the_batch() {
        let mut server = server();
        let before = structured(&mut server, "get_castle", json!({}));
        let result = structured(
            &mut server,
            "apply_operations",
            json!({
                "operations": [
                    sample_wall("south"),
                    {
                        "op": "add_arrow_loop",
                        "wall_ref": "south",
                        "t_permille": 500,
                        "sill_height_mm": 1_700
                    },
                    {
                        "op": "create_tower",
                        "ref_id": "angle",
                        "name": "Angle tower",
                        "centre_mm": [10_000, 6_000],
                        "radius_mm": 3_000,
                        "height_mm": 12_000
                    }
                ]
            }),
        );
        assert_eq!(result["applied_count"], 3);
        let south_id = result["created_ids"]["south"].as_u64().unwrap();
        let angle_id = result["created_ids"]["angle"].as_u64().unwrap();
        assert_eq!(south_id, 12);
        assert_eq!(angle_id, 14);

        let after = structured(&mut server, "get_castle", json!({}));
        assert_eq!(
            after["element_count"].as_u64().unwrap(),
            before["element_count"].as_u64().unwrap() + 2
        );
        assert_eq!(
            after["opening_count"].as_u64().unwrap(),
            before["opening_count"].as_u64().unwrap() + 1
        );
        let openings = after["openings"].as_array().unwrap();
        let added = openings
            .iter()
            .find(|opening| opening["wall_id"] == south_id)
            .expect("new opening attached to new wall");
        assert_eq!(added["t_permille"], 500);
        assert_eq!(added["height_mm"], 1_500);
        assert_eq!(added["outer_slit_mm"], 140);
        assert_eq!(added["inner_splay_width_mm"], 1_100);
        assert_eq!(server.document().opening_count(), 3);
    }

    #[test]
    fn apply_batch_can_target_a_wall_by_numeric_id() {
        let mut server = server();
        let result = structured(
            &mut server,
            "apply_operations",
            json!({
                "operations": [{
                    "op": "add_arrow_loop",
                    "wall_id": 1,
                    "t_permille": 600,
                    "sill_height_mm": 2_000,
                    "height_mm": 1_200,
                    "outer_slit_mm": 120,
                    "inner_splay_width_mm": 1_300
                }]
            }),
        );
        assert_eq!(result["applied_count"], 1);
        let castle = structured(&mut server, "get_castle", json!({}));
        assert_eq!(castle["opening_count"], 3);
        let opening = castle["openings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|opening| opening["id"] == 12)
            .unwrap();
        assert_eq!(opening["wall_id"], 1);
        assert_eq!(opening["outer_slit_mm"], 120);
    }

    #[test]
    fn oversized_batch_fails_and_rolls_back() {
        let mut server = server();
        let before = structured(&mut server, "get_castle", json!({}));
        let operations: Vec<Value> = (0..=MAX_BATCH_OPS)
            .map(|_| json!({ "op": "delete_element", "id": 1 }))
            .collect();
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({ "operations": operations }),
        );
        assert!(message.contains("maximum is"));
        let after = structured(&mut server, "get_castle", json!({}));
        assert_eq!(before, after);
    }

    #[test]
    fn unknown_wall_ref_rolls_back_the_whole_batch() {
        let mut server = server();
        let before = structured(&mut server, "get_castle", json!({}));
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({
                "operations": [
                    sample_wall("north"),
                    {
                        "op": "add_arrow_loop",
                        "wall_ref": "missing",
                        "t_permille": 250,
                        "sill_height_mm": 1_500
                    }
                ]
            }),
        );
        assert!(message.contains("unknown wall_ref"));
        let after = structured(&mut server, "get_castle", json!({}));
        assert_eq!(before, after);
        assert_eq!(server.document().len(), 3, "wall was not committed");
    }

    #[test]
    fn failed_later_operation_rolls_back_earlier_ones() {
        let mut server = server();
        let before = structured(&mut server, "get_castle", json!({}));
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({
                "operations": [
                    sample_wall("north"),
                    { "op": "delete_element", "id": 999_999 }
                ]
            }),
        );
        assert!(message.contains("unknown element id"));
        let after = structured(&mut server, "get_castle", json!({}));
        assert_eq!(before, after);
    }

    #[test]
    fn arrow_loop_onto_a_tower_is_rejected() {
        let mut server = server();
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({
                "operations": [{
                    "op": "add_arrow_loop",
                    "wall_id": 3,
                    "t_permille": 500,
                    "sill_height_mm": 1_500
                }]
            }),
        );
        assert!(message.contains("not a wall run"));
    }

    #[test]
    fn arrow_loop_needs_a_wall_reference() {
        let mut server = server();
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({
                "operations": [{
                    "op": "add_arrow_loop",
                    "t_permille": 500,
                    "sill_height_mm": 1_500
                }]
            }),
        );
        assert!(message.contains("wall_id or wall_ref"));
    }

    #[test]
    fn malformed_arguments_are_tool_errors_and_do_not_mutate() {
        let mut server = server();
        let before = structured(&mut server, "get_castle", json!({}));
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({ "operations": [{ "op": "create_wall_run", "name": "x" }] }),
        );
        assert!(message.contains("invalid arguments"));
        let after = structured(&mut server, "get_castle", json!({}));
        assert_eq!(before, after);
    }

    #[test]
    fn names_are_validated_at_the_tool_boundary() {
        let mut server = server();
        let before = structured(&mut server, "get_castle", json!({}));

        let too_long = "a".repeat(MAX_NAME_CHARS + 1);
        let mut wall = sample_wall("north");
        wall["name"] = Value::String(too_long);
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({ "operations": [wall] }),
        );
        assert!(message.contains("maximum is 64"));

        let mut wall = sample_wall("north");
        wall["name"] = Value::String("bad\nname".into());
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({ "operations": [wall] }),
        );
        assert!(message.contains("control characters"));

        let after = structured(&mut server, "get_castle", json!({}));
        assert_eq!(before, after);
    }

    #[test]
    fn names_are_trimmed_when_applied() {
        let mut server = server();
        let mut wall = sample_wall("north");
        wall["name"] = Value::String("  Trimmed wall  ".into());
        structured(
            &mut server,
            "apply_operations",
            json!({ "operations": [wall] }),
        );
        let castle = structured(&mut server, "get_castle", json!({}));
        let added = castle["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|element| element["name"] == "Trimmed wall")
            .unwrap();
        assert_eq!(added["name"], "Trimmed wall");
    }

    #[test]
    fn duplicate_ref_ids_are_rejected_atomically() {
        let mut server = server();
        let before = structured(&mut server, "get_castle", json!({}));
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({
                "operations": [sample_wall("same"), sample_wall("same")]
            }),
        );
        assert!(message.contains("duplicate ref_id"));
        let after = structured(&mut server, "get_castle", json!({}));
        assert_eq!(before, after);
    }

    #[test]
    fn bad_geometry_is_rejected() {
        let mut server = server();
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({
                "operations": [{
                    "op": "create_wall_run",
                    "name": "Flat",
                    "start_mm": [0, 0],
                    "end_mm": [0, 0],
                    "height_mm": 5_000,
                    "thickness_mm": 1_500
                }]
            }),
        );
        assert!(message.contains("must differ"));
    }

    #[test]
    fn extreme_numeric_inputs_are_rejected_cleanly() {
        let mut server = server();
        let before = structured(&mut server, "get_castle", json!({}));

        let cases = [
            (
                json!({
                    "op": "create_wall_run",
                    "name": "Huge",
                    "start_mm": [0, 0],
                    "end_mm": [9_000_000_000i64, 0],
                    "height_mm": 9_000,
                    "thickness_mm": 2_000
                }),
                "end_mm",
            ),
            (
                json!({
                    "op": "create_wall_run",
                    "name": "Tall",
                    "start_mm": [0, 0],
                    "end_mm": [10_000, 0],
                    "height_mm": 900_000_000i64,
                    "thickness_mm": 2_000
                }),
                "height_mm",
            ),
            (
                json!({
                    "op": "create_wall_run",
                    "name": "Thin",
                    "start_mm": [0, 0],
                    "end_mm": [10_000, 0],
                    "height_mm": 9_000,
                    "thickness_mm": 1
                }),
                "thickness_mm",
            ),
            (
                json!({
                    "op": "create_tower",
                    "name": "Wide",
                    "centre_mm": [0, 0],
                    "radius_mm": 400_000_000i64,
                    "height_mm": 12_000
                }),
                "radius_mm",
            ),
            (
                json!({
                    "op": "create_tower",
                    "name": "Endless",
                    "centre_mm": [0, 0],
                    "radius_mm": 3_000,
                    "height_mm": i64::MAX
                }),
                "height_mm",
            ),
            (
                json!({
                    "op": "create_tower",
                    "name": "Far",
                    "centre_mm": [500_001, 0],
                    "radius_mm": 3_000,
                    "height_mm": 12_000
                }),
                "centre_mm",
            ),
            (
                json!({
                    "op": "add_arrow_loop",
                    "wall_id": 1,
                    "t_permille": 1000,
                    "sill_height_mm": 1_500
                }),
                "t_permille",
            ),
            (
                json!({
                    "op": "add_arrow_loop",
                    "wall_id": 1,
                    "t_permille": 500,
                    "sill_height_mm": 1_500,
                    "height_mm": i64::MAX
                }),
                "height_mm",
            ),
        ];

        for (operation, field) in cases {
            let message = tool_error_message(
                &mut server,
                "apply_operations",
                json!({ "operations": [operation] }),
            );
            assert!(
                message.contains(field),
                "expected error mentioning {field}, got {message}"
            );
        }

        let after = structured(&mut server, "get_castle", json!({}));
        assert_eq!(
            before, after,
            "rejected batches must not mutate the document"
        );
    }

    #[test]
    fn unsafe_names_and_ref_ids_are_rejected() {
        let mut server = server();
        let before = structured(&mut server, "get_castle", json!({}));

        let mut wall = sample_wall("north");
        wall["name"] = Value::String("curtain \u{202E}spoiler".into());
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({ "operations": [wall] }),
        );
        assert!(message.contains("bidi"), "got {message}");

        let mut wall = sample_wall("north wall");
        wall["ref_id"] = Value::String("north wall".into());
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({ "operations": [wall] }),
        );
        assert!(message.contains("ref_id"), "got {message}");

        let mut wall = sample_wall("north");
        wall["ref_id"] = Value::String("a".repeat(65));
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({ "operations": [wall] }),
        );
        assert!(message.contains("ref_id"), "got {message}");

        let after = structured(&mut server, "get_castle", json!({}));
        assert_eq!(before, after);
    }

    #[test]
    fn element_budget_is_enforced_and_rolls_back() {
        let mut server = McpServer::new(true);
        let fill = |server: &mut McpServer, count: usize| {
            let operations: Vec<Value> = (0..count)
                .map(|index| {
                    let x = ((index % 16) as i64) * 30_000 - 240_000;
                    let z = ((index / 16) as i64) * 30_000 - 240_000;
                    json!({
                        "op": "create_wall_run",
                        "name": format!("Wall {index}"),
                        "start_mm": [x, z],
                        "end_mm": [x, z + 10_000],
                        "height_mm": 5_000,
                        "thickness_mm": 1_000
                    })
                })
                .collect();
            structured(
                server,
                "apply_operations",
                json!({ "operations": operations }),
            )
        };
        let first = fill(&mut server, 256);
        assert_eq!(first["applied_count"], 256);
        let second = fill(&mut server, 256);
        assert_eq!(second["applied_count"], 256);
        assert_eq!(server.document().len(), 512);

        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({
                "operations": [{
                    "op": "create_wall_run",
                    "name": "One too many",
                    "start_mm": [0, 0],
                    "end_mm": [10_000, 0],
                    "height_mm": 5_000,
                    "thickness_mm": 1_000
                }]
            }),
        );
        assert!(message.contains("maximum is 512"), "got {message}");
        assert_eq!(server.document().len(), 512);
    }

    #[test]
    fn ids_are_never_reused_after_deleting_the_highest_element() {
        let mut server = server();
        let before = structured(&mut server, "get_castle", json!({}));
        let highest = before["elements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|element| element["id"].as_u64().unwrap())
            .max()
            .unwrap();

        structured(
            &mut server,
            "apply_operations",
            json!({ "operations": [{ "op": "delete_element", "id": highest }] }),
        );

        let result = structured(
            &mut server,
            "apply_operations",
            json!({ "operations": [sample_wall("after-delete")] }),
        );
        let new_id = result["created_ids"]["after-delete"].as_u64().unwrap();
        assert!(
            new_id > highest,
            "new id {new_id} must be greater than deleted id {highest}"
        );

        // A second deletion of the new highest must still not reuse the id.
        structured(
            &mut server,
            "apply_operations",
            json!({ "operations": [{ "op": "delete_element", "id": new_id }] }),
        );
        let result = structured(
            &mut server,
            "apply_operations",
            json!({ "operations": [sample_wall("after-second-delete")] }),
        );
        let newer_id = result["created_ids"]["after-second-delete"]
            .as_u64()
            .unwrap();
        assert!(newer_id > new_id);
    }

    #[test]
    fn delete_removes_attached_openings() {
        let mut server = server();
        let castle = structured(
            &mut server,
            "apply_operations",
            json!({ "operations": [{ "op": "delete_element", "id": 1 }] }),
        );
        assert_eq!(castle["applied_count"], 1);
        let after = structured(&mut server, "get_castle", json!({}));
        assert_eq!(after["element_count"], 2);
        assert_eq!(after["opening_count"], 0);
        assert!(after["elements"]
            .as_array()
            .unwrap()
            .iter()
            .all(|element| element["id"] != 1));
    }

    #[test]
    fn empty_batch_is_a_successful_no_op() {
        let mut server = server();
        let before = structured(&mut server, "get_castle", json!({}));
        let result = structured(&mut server, "apply_operations", json!({ "operations": [] }));
        assert_eq!(result["applied_count"], 0);
        assert!(result["created_ids"].as_object().unwrap().is_empty());
        let after = structured(&mut server, "get_castle", json!({}));
        assert_eq!(before["document_hash"], after["document_hash"]);
    }

    #[test]
    fn resources_list_and_read() {
        let mut server = server();
        let response = call(&mut server, 5, "resources/list", json!({}));
        let resources = response["result"]["resources"].as_array().unwrap();
        assert_eq!(resources.len(), 6);
        assert_eq!(resources[0]["uri"], "keep://castle/document");
        assert_eq!(resources[1]["uri"], "keep://castle/save");
        assert_eq!(resources[1]["mimeType"], "application/json");
        assert_eq!(resources[2]["mimeType"], "image/svg+xml");
        assert_eq!(resources[4]["uri"], "keep://castle/assessment");
        assert_eq!(resources[5]["uri"], "keep://styles/edwardian_welsh");
        assert_eq!(resources[4]["mimeType"], "application/json");

        let document = call(
            &mut server,
            6,
            "resources/read",
            json!({ "uri": "keep://castle/document" }),
        );
        let contents = &document["result"]["contents"][0];
        assert_eq!(contents["mimeType"], "application/json");
        let parsed: Value = serde_json::from_str(contents["text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["element_count"], 3);

        for uri in ["keep://castle/plan/top", "keep://castle/plan/elevation"] {
            let response = call(&mut server, 7, "resources/read", json!({ "uri": uri }));
            let contents = &response["result"]["contents"][0];
            assert_eq!(contents["mimeType"], "image/svg+xml");
            assert!(contents["text"].as_str().unwrap().starts_with("<svg "));
        }

        let assessment = call(
            &mut server,
            9,
            "resources/read",
            json!({ "uri": "keep://castle/assessment" }),
        );
        let contents = &assessment["result"]["contents"][0];
        assert_eq!(contents["mimeType"], "application/json");
        let parsed: Value = serde_json::from_str(contents["text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["style_id"], "edwardian_welsh");
        assert!(parsed["passed"].is_boolean());

        let style = call(
            &mut server,
            10,
            "resources/read",
            json!({ "uri": "keep://styles/edwardian_welsh" }),
        );
        let contents = &style["result"]["contents"][0];
        assert_eq!(contents["mimeType"], "application/json");
        let parsed: Value = serde_json::from_str(contents["text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["id"], "edwardian_welsh");
    }

    #[test]
    fn unknown_resource_is_an_error() {
        let mut server = server();
        let response = call(
            &mut server,
            8,
            "resources/read",
            json!({ "uri": "keep://castle/not-a-resource" }),
        );
        assert_eq!(response["error"]["code"], code::RESOURCE_NOT_FOUND);
    }

    #[test]
    fn empty_document_server_has_no_elements_but_renders() {
        let mut server = McpServer::new(true);
        let castle = structured(&mut server, "get_castle", json!({}));
        assert_eq!(castle["element_count"], 0);
        assert_eq!(castle["opening_count"], 0);
        let plan = structured(&mut server, "render_plan", json!({ "view": "top_plan" }));
        assert!(plan["svg"].as_str().unwrap().starts_with("<svg "));
    }

    #[test]
    fn handle_line_round_trips_a_request() {
        let mut server = server();
        let line = server
            .handle_line(r#"{"jsonrpc":"2.0","id":42,"method":"ping"}"#)
            .unwrap();
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["id"], 42);
        assert_eq!(value["result"], json!({}));
    }

    fn is_uuid_v7(id: &str) -> bool {
        let groups: Vec<&str> = id.split('-').collect();
        groups.iter().map(|group| group.len()).collect::<Vec<_>>() == [8, 4, 4, 4, 12]
            && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
            && id.as_bytes().get(14) == Some(&b'7')
            && matches!(id.as_bytes().get(19), Some(b'8' | b'9' | b'a' | b'b'))
    }

    #[test]
    fn undo_restores_exact_document_and_layout_hashes() {
        let mut server = server();
        let before = structured(&mut server, "get_castle", json!({}));
        let applied = structured(
            &mut server,
            "apply_operations",
            json!({ "operations": [sample_wall("undoable")] }),
        );
        assert_ne!(applied["document_hash"], before["document_hash"]);

        let undo = structured(&mut server, "undo", json!({}));
        assert_eq!(undo["compensated_seq"], 1);
        assert_eq!(undo["seq"], 2);
        assert_eq!(undo["document_hash"], before["document_hash"]);
        assert_eq!(undo["layout_hash"], before["layout_hash"]);
        assert_eq!(undo["block_count"], before["block_count"]);
        assert_eq!(undo["chunk_count"], before["chunk_count"]);
        assert_eq!(server.document().len(), 3);
    }

    #[test]
    fn undo_on_empty_history_is_a_tool_error() {
        let mut server = McpServer::new(true);
        let before = structured(&mut server, "get_castle", json!({}));
        let message = tool_error_message(&mut server, "undo", json!({}));
        assert!(message.contains("no command left to undo"), "got {message}");
        let after = structured(&mut server, "get_castle", json!({}));
        assert_eq!(before, after);
    }

    #[test]
    fn get_history_shows_applied_and_compensating_entries() {
        let mut server = server();
        structured(
            &mut server,
            "apply_operations",
            json!({ "operations": [sample_wall("history")] }),
        );
        structured(&mut server, "undo", json!({}));

        let history = structured(&mut server, "get_history", json!({}));
        assert_eq!(history["count"], 2);
        let entries = history["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0]["seq"], 1);
        assert_eq!(entries[0]["author"], "agent:mcp-offline");
        assert_eq!(entries[0]["summary"], "+1 wall");
        assert_eq!(entries[0]["touched_elements"], json!([12]));
        assert_eq!(entries[0]["undone"], true);
        let applied_id = entries[0]["command_id"].as_str().unwrap();
        assert!(is_uuid_v7(applied_id), "not a UUIDv7: {applied_id}");

        assert_eq!(entries[1]["seq"], 2);
        assert_eq!(entries[1]["author"], "agent:mcp-offline");
        assert_eq!(entries[1]["summary"], "Undo command #1");
        assert_eq!(entries[1]["touched_elements"], json!([12]));
        assert_eq!(entries[1]["undone"], false);
        let undo_id = entries[1]["command_id"].as_str().unwrap();
        assert!(is_uuid_v7(undo_id), "not a UUIDv7: {undo_id}");
        assert_ne!(applied_id, undo_id);
    }

    #[test]
    fn get_history_on_a_fresh_server_is_empty() {
        let mut server = McpServer::new(true);
        let history = structured(&mut server, "get_history", json!({}));
        assert_eq!(history["count"], 0);
        assert_eq!(history["entries"], json!([]));
    }

    #[test]
    fn get_assessment_flags_slenderness_and_a_suggested_fix_repairs_it() {
        let mut server = McpServer::new(true);
        structured(
            &mut server,
            "apply_operations",
            json!({
                "operations": [{
                    "op": "create_wall_run",
                    "name": "Thin wall",
                    "start_mm": [-6_000, 0],
                    "end_mm": [6_000, 0],
                    "height_mm": 12_000,
                    "thickness_mm": 1_000
                }]
            }),
        );

        let before = structured(&mut server, "get_assessment", json!({}));
        assert_eq!(before["style_id"], "edwardian_welsh");
        assert_eq!(before["passed"], false);
        let finding = before["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["code"] == "wall_too_slender")
            .expect("slenderness finding");
        assert_eq!(finding["severity"], "error");
        assert!(finding["source_note"]
            .as_str()
            .unwrap()
            .contains("Caernarfon"));
        let fix = finding["suggested_fix"].clone();
        assert_eq!(fix["op"], "modify_element");
        assert!(fix["thickness_mm"].as_i64().unwrap() >= 2_667);

        // Applying the one-click fix through the normal batch API repairs it.
        structured(
            &mut server,
            "apply_operations",
            json!({ "operations": [fix] }),
        );
        let after = structured(&mut server, "get_assessment", json!({}));
        assert_eq!(after["passed"], true, "findings: {}", after["findings"]);
        assert_eq!(after["score"], 100);
        assert!(after["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| finding["code"] != "wall_too_slender"));
    }

    #[test]
    fn list_and_get_styles() {
        let mut server = server();
        let styles = structured(&mut server, "list_styles", json!({}));
        let styles = styles["styles"].as_array().unwrap();
        assert_eq!(styles.len(), 1);
        assert_eq!(styles[0]["id"], "edwardian_welsh");
        assert_eq!(styles[0]["course_height_mm"], 450);
        assert!(styles[0]["source_notes"].as_array().unwrap().len() >= 4);

        let style = structured(&mut server, "get_style", json!({ "id": "edwardian_welsh" }));
        assert_eq!(style["style"]["id"], "edwardian_welsh");
        assert_eq!(style["style"]["min_wall_thickness_mm"], 1_800);
        assert_eq!(style["style"]["max_slenderness_ratio"], 4.5);

        let message = tool_error_message(&mut server, "get_style", json!({ "id": "gothic" }));
        assert!(message.contains("unknown style id"), "got {message}");

        let long_id = "x".repeat(200);
        let message = tool_error_message(&mut server, "get_style", json!({ "id": long_id }));
        assert_eq!(
            message.matches('x').count(),
            64,
            "unknown-style id must be truncated to 64 chars: {message}"
        );

        let message = tool_error_message(&mut server, "get_style", json!({}));
        assert!(message.contains("invalid arguments"), "got {message}");
    }

    #[test]
    fn get_site_reports_the_active_site_and_catalogue() {
        let mut server = server();
        let site = structured(&mut server, "get_site", json!({}));
        assert_eq!(site["site"]["id"], "m0a_promontory");
        assert_eq!(site["site"]["name"], "River Promontory");
        let available = site["available_sites"].as_array().unwrap();
        assert_eq!(available.len(), 3);
        let ids: Vec<&str> = available
            .iter()
            .map(|site| site["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            vec!["m0a_promontory", "coastal_headland", "lake_island"]
        );

        let mut lake = McpServer::with_start("lake_island", CastleDocument::new())
            .expect("lake_island is curated");
        let site = structured(&mut lake, "get_site", json!({}));
        assert_eq!(site["site"]["id"], "lake_island");
        assert_eq!(site["site"]["name"], "Lake Island");
    }

    #[test]
    fn create_gatehouse_ditch_and_modify_element() {
        let mut server = McpServer::new(true);
        let applied = structured(
            &mut server,
            "apply_operations",
            json!({
                "operations": [
                    {
                        "op": "create_gatehouse",
                        "ref_id": "gate",
                        "name": "Great gatehouse",
                        "centre_mm": [0, 0],
                        "orientation_deg": 0
                    },
                    {
                        "op": "create_ditch",
                        "ref_id": "ditch",
                        "name": "Approach ditch",
                        "start_mm": [-20_000, -10_000],
                        "end_mm": [-20_000, 10_000],
                        "width_mm": 6_000,
                        "depth_mm": 2_600
                    },
                    {
                        "op": "create_wall_run",
                        "ref_id": "wall",
                        "name": "Curtain",
                        "start_mm": [5_000, 0],
                        "end_mm": [15_000, 0],
                        "height_mm": 9_000,
                        "thickness_mm": 2_400
                    }
                ]
            }),
        );
        assert_eq!(applied["applied_count"], 3);
        assert_eq!(applied["created_ids"]["gate"], 1);
        assert_eq!(applied["created_ids"]["ditch"], 2);
        let wall_id = applied["created_ids"]["wall"].as_u64().unwrap();
        assert_eq!(wall_id, 3);

        let castle = structured(&mut server, "get_castle", json!({}));
        let kinds: Vec<&str> = castle["elements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|element| element["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, vec!["gatehouse", "ditch", "wall"]);
        assert_eq!(castle["elements"][0]["name"], "Great gatehouse");
        assert_eq!(castle["elements"][1]["name"], "Approach ditch");

        let modified = structured(
            &mut server,
            "apply_operations",
            json!({
                "operations": [{
                    "op": "modify_element",
                    "id": wall_id,
                    "height_mm": 7_000,
                    "thickness_mm": 2_000
                }]
            }),
        );
        assert_eq!(modified["applied_count"], 1);
        let castle = structured(&mut server, "get_castle", json!({}));
        let wall = castle["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|element| element["id"] == wall_id)
            .unwrap();
        assert_eq!(wall["height_mm"], 7_000);
        assert_eq!(wall["thickness_mm"], 2_000);
    }

    #[test]
    fn save_resource_round_trips_through_session() {
        let mut server = server();
        structured(
            &mut server,
            "apply_operations",
            json!({ "operations": [sample_wall("saved")] }),
        );

        let response = call(
            &mut server,
            12,
            "resources/read",
            json!({ "uri": "keep://castle/save" }),
        );
        let contents = &response["result"]["contents"][0];
        assert_eq!(contents["uri"], "keep://castle/save");
        assert_eq!(contents["mimeType"], "application/json");
        let text = contents["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["schema_version"], 1);
        assert_eq!(parsed["command_log"].as_array().unwrap().len(), 1);

        let loaded = Session::from_save_json(text).expect("save resource round-trips");
        assert_eq!(
            loaded.document().document_hash(),
            server.document().document_hash()
        );
        assert_eq!(
            loaded.build_scene().layout_hash,
            server.session().build_scene().layout_hash
        );
        assert_eq!(loaded.history().len(), 1);
    }

    // -------------------------------------------------------------------
    // Proposals and permissions (M1.3 wave 2a)
    // -------------------------------------------------------------------

    fn proposal_server(permission: AgentPermission) -> McpServer {
        McpServer::with_permission(
            keep_core::session::DEFAULT_SITE_ID,
            SiteTerrain::m0a_promontory(),
            CastleDocument::new(),
            permission,
        )
    }

    #[test]
    fn proposal_lifecycle_leaves_the_live_document_untouched() {
        let mut server = proposal_server(AgentPermission::ProposalOnly);
        let before = structured(&mut server, "get_castle", json!({}));

        let begun = structured(
            &mut server,
            "begin_proposal",
            json!({ "title": "North curtain" }),
        );
        assert_eq!(begun["proposal"]["id"], "prop_0001");
        assert_eq!(begun["proposal"]["status"], "draft");
        assert_eq!(begun["proposal"]["author"]["kind"], "agent");
        assert_eq!(begun["proposal"]["author"]["id"], OFFLINE_AUTHOR_ID);
        assert_eq!(begun["proposal"]["ops"], json!([]));

        let staged = structured(
            &mut server,
            "stage_proposal_ops",
            json!({
                "proposal_id": "prop_0001",
                "operations": [sample_wall("north")]
            }),
        );
        assert_eq!(staged["proposal"]["ops"].as_array().unwrap().len(), 1);
        assert_eq!(staged["proposal"]["status"], "draft");
        assert!(staged["proposal"]["structure_after"].is_number());
        assert!(staged["proposal"]["semantic_diff"]
            .as_str()
            .unwrap()
            .contains("wall"));

        let submitted = structured(
            &mut server,
            "submit_proposal",
            json!({
                "proposal_id": "prop_0001",
                "summary": "A new north curtain"
            }),
        );
        assert_eq!(submitted["proposal"]["status"], "submitted");
        assert_eq!(submitted["proposal"]["summary"], "A new north curtain");

        let listed = structured(&mut server, "list_proposals", json!({}));
        assert_eq!(listed["count"], 1);
        assert_eq!(listed["proposals"][0]["id"], "prop_0001");
        assert_eq!(listed["proposals"][0]["status"], "submitted");

        // The overlay is not the castle: nothing was applied or logged.
        let after = structured(&mut server, "get_castle", json!({}));
        assert_eq!(before["document_hash"], after["document_hash"]);
        assert_eq!(server.document().len(), 0);
        assert!(server.session().history().is_empty());
    }

    #[test]
    fn proposal_only_refuses_direct_edits_with_guidance() {
        let mut server = proposal_server(AgentPermission::ProposalOnly);
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({ "operations": [sample_wall("north")] }),
        );
        assert!(message.contains("proposal-only"), "got {message}");
        assert!(message.contains("begin_proposal"), "got {message}");
        assert!(message.contains("in-game"), "got {message}");

        let message = tool_error_message(&mut server, "undo", json!({}));
        assert!(message.contains("proposal-only"), "got {message}");
        assert_eq!(server.document().len(), 0);
    }

    #[test]
    fn read_only_refuses_every_write_and_proposal_tool() {
        let mut server = proposal_server(AgentPermission::ReadOnly);

        // Reads and renders still work.
        structured(&mut server, "get_castle", json!({}));
        structured(&mut server, "list_elements", json!({}));
        structured(&mut server, "render_plan", json!({}));
        let listed = structured(&mut server, "list_proposals", json!({}));
        assert_eq!(listed["count"], 0);

        for (tool, arguments) in [
            (
                "apply_operations",
                json!({ "operations": [sample_wall("north")] }),
            ),
            ("undo", json!({})),
            ("begin_proposal", json!({ "title": "Nope" })),
            (
                "stage_proposal_ops",
                json!({ "proposal_id": "prop_0001", "operations": [] }),
            ),
            (
                "submit_proposal",
                json!({ "proposal_id": "prop_0001", "summary": "Nope" }),
            ),
        ] {
            let message = tool_error_message(&mut server, tool, arguments);
            assert!(
                message.contains("read-only"),
                "{tool} should be refused, got {message}"
            );
        }

        let message = tool_error_message(&mut server, "render_view", json!({}));
        assert!(message.contains("requires --live mode"), "got {message}");
        assert!(message.contains("render_plan offline"), "got {message}");
        assert_eq!(server.document().len(), 0);
        assert!(server.session().proposals().next().is_none());
    }

    #[test]
    fn direct_edit_can_still_propose_and_apply() {
        let mut server = proposal_server(AgentPermission::DirectEdit);
        assert_eq!(server.permission(), AgentPermission::DirectEdit);
        let begun = structured(
            &mut server,
            "begin_proposal",
            json!({ "title": "Direct proposal" }),
        );
        assert_eq!(begun["proposal"]["status"], "draft");
        let applied = structured(
            &mut server,
            "apply_operations",
            json!({ "operations": [sample_wall("north")] }),
        );
        assert_eq!(applied["applied_count"], 1);
    }

    #[test]
    fn proposal_errors_are_tool_errors() {
        let mut server = proposal_server(AgentPermission::ProposalOnly);
        let message = tool_error_message(
            &mut server,
            "stage_proposal_ops",
            json!({ "proposal_id": "prop_9999", "operations": [] }),
        );
        assert!(message.contains("unknown proposal"), "got {message}");

        structured(&mut server, "begin_proposal", json!({ "title": "Empty" }));
        let message = tool_error_message(
            &mut server,
            "submit_proposal",
            json!({ "proposal_id": "prop_0001", "summary": "Nothing" }),
        );
        assert!(message.contains("no staged operations"), "got {message}");

        let message = tool_error_message(&mut server, "begin_proposal", json!({}));
        assert!(message.contains("invalid arguments"), "got {message}");
    }

    // -------------------------------------------------------------------
    // Live HTTP JSON-RPC mode
    // -------------------------------------------------------------------

    struct MockServer {
        url: String,
        requests: Arc<Mutex<Vec<String>>>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl MockServer {
        fn finish(mut self) {
            if let Some(handle) = self.handle.take() {
                handle.join().expect("mock server thread");
            }
        }
    }

    /// A tiny loopback HTTP/1.1 server: one JSON-RPC response per connection.
    fn spawn_mock(responses: Vec<Value>) -> MockServer {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind mock server");
        let port = listener.local_addr().unwrap().port();
        let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept mock request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    let read = reader.read_line(&mut line).expect("read request line");
                    if read == 0 || line == "\r\n" {
                        break;
                    }
                    head.push_str(&line);
                }
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).expect("read request body");
                recorded
                    .lock()
                    .unwrap()
                    .push(format!("{head}\r\n{}", String::from_utf8_lossy(&body)));

                let payload = serde_json::to_string(&response).unwrap();
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    payload.len(),
                    payload
                );
                stream.write_all(reply.as_bytes()).unwrap();
                stream.flush().unwrap();
            }
        });
        MockServer {
            url: format!("http://127.0.0.1:{port}"),
            requests,
            handle: Some(handle),
        }
    }

    fn header_value(request: &str, name: &str) -> Option<String> {
        request.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name)
                .then(|| value.trim().to_string())
        })
    }

    #[test]
    fn live_client_round_trips_tools_with_auth_and_host_headers() {
        let mock = spawn_mock(vec![
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": live_castle_result("0123456789abcdef")
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": { "proposal": { "id": "prop_0001", "status": "draft" } }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "result": {
                    "width": 768,
                    "height": 512,
                    "mime_type": "image/png",
                    "image_base64": "aGVsbG8="
                }
            }),
        ]);
        let expected_host = mock.url.strip_prefix("http://").unwrap().to_string();
        let requests = Arc::clone(&mock.requests);

        let mut server =
            McpServer::live(&mock.url, "secret-token", AgentPermission::DirectEdit).unwrap();
        assert!(server.is_live());

        let castle = structured(&mut server, "get_castle", json!({}));
        assert_eq!(castle["document_hash"], "0123456789abcdef");

        let begun = structured(
            &mut server,
            "begin_proposal",
            json!({ "title": "Live proposal" }),
        );
        assert_eq!(begun["proposal"]["id"], "prop_0001");

        let rendered = call_tool(&mut server, "render_view", json!({ "yaw_deg": 45.0 }));
        assert_eq!(rendered["isError"], false);
        assert_eq!(rendered["content"][0]["type"], "image");
        assert_eq!(rendered["content"][0]["data"], "aGVsbG8=");
        assert_eq!(rendered["content"][0]["mimeType"], "image/png");
        assert_eq!(rendered["structuredContent"]["width"], 768);
        assert_eq!(rendered["structuredContent"]["height"], 512);

        mock.finish();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        for request in requests.iter() {
            assert!(
                request.starts_with("POST /rpc HTTP/1.1\r\n"),
                "unexpected request line: {request}"
            );
            assert_eq!(
                header_value(request, "Host").as_deref(),
                Some(expected_host.as_str())
            );
            assert_eq!(
                header_value(request, "Authorization").as_deref(),
                Some("Bearer secret-token")
            );
            assert_eq!(
                header_value(request, "Content-Type").as_deref(),
                Some("application/json")
            );
            assert_eq!(
                header_value(request, "Connection").as_deref(),
                Some("close")
            );
        }
        assert!(requests[0].contains("\"method\":\"get_castle\""));
        assert!(requests[1].contains("\"method\":\"begin_proposal\""));
        assert!(requests[2].contains("\"method\":\"render_view\""));
    }

    #[test]
    fn live_mode_rejects_non_loopback_urls() {
        for url in [
            "http://example.com:8765",
            "http://192.168.1.20:8765",
            "https://127.0.0.1:8765",
        ] {
            assert!(
                McpServer::live(url, "token", AgentPermission::DirectEdit).is_err(),
                "accepted {url}"
            );
        }
        assert!(LiveClient::connect("http://localhost:1", "token").is_ok());
    }

    #[test]
    fn live_mode_enforces_permissions_before_network_calls() {
        let mut server =
            McpServer::live("http://127.0.0.1:1", "token", AgentPermission::ProposalOnly).unwrap();
        let message = tool_error_message(
            &mut server,
            "apply_operations",
            json!({ "operations": [sample_wall("north")] }),
        );
        assert!(message.contains("proposal-only"), "got {message}");

        let mut server =
            McpServer::live("http://127.0.0.1:1", "token", AgentPermission::ReadOnly).unwrap();
        let message = tool_error_message(&mut server, "begin_proposal", json!({ "title": "Nope" }));
        assert!(message.contains("read-only"), "got {message}");
    }

    #[test]
    fn live_mode_surfaces_control_server_errors_as_tool_errors() {
        let mock = spawn_mock(vec![json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32601, "message": "unknown method" }
        })]);
        let mut server = McpServer::live(&mock.url, "token", AgentPermission::DirectEdit).unwrap();
        let message = tool_error_message(&mut server, "get_castle", json!({}));
        assert!(message.contains("control server error"), "got {message}");
        assert!(message.contains("unknown method"), "got {message}");
        mock.finish();
    }

    fn live_castle_result(document_hash: &str) -> Value {
        json!({
            "document_hash": document_hash,
            "layout_hash": "fedcba9876543210",
            "element_count": 1,
            "opening_count": 0,
            "block_count": 42,
            "chunk_count": 1,
            "elements": [],
            "openings": []
        })
    }

    #[test]
    fn live_mode_strips_an_agent_supplied_identity() {
        let mock = spawn_mock(vec![json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": live_castle_result("0123456789abcdef")
        })]);
        let requests = Arc::clone(&mock.requests);
        let mut server = McpServer::live(&mock.url, "token", AgentPermission::DirectEdit).unwrap();
        let castle = structured(
            &mut server,
            "get_castle",
            json!({ "agent_id": "human:owner" }),
        );
        assert_eq!(castle["document_hash"], "0123456789abcdef");
        mock.finish();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(
            !requests[0].contains("agent_id"),
            "agent_id must never be forwarded: {}",
            requests[0]
        );
    }

    #[test]
    fn live_resources_read_forwards_to_the_game_not_the_empty_offline_session() {
        let mock = spawn_mock(vec![
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": live_castle_result("0123456789abcdef")
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": {
                    "view": "top_plan",
                    "document_hash": "0123456789abcdef",
                    "mime_type": "image/svg+xml",
                    "svg": "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"
                }
            }),
        ]);
        let requests = Arc::clone(&mock.requests);
        let mut server = McpServer::live(&mock.url, "token", AgentPermission::DirectEdit).unwrap();

        let document = call(
            &mut server,
            20,
            "resources/read",
            json!({ "uri": "keep://castle/document" }),
        );
        let contents = &document["result"]["contents"][0];
        assert_eq!(contents["mimeType"], "application/json");
        let parsed: Value = serde_json::from_str(contents["text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["document_hash"], "0123456789abcdef");
        assert_eq!(parsed["block_count"], 42);
        assert_eq!(
            parsed["element_count"], 1,
            "must not return the empty offline session"
        );

        let plan = call(
            &mut server,
            21,
            "resources/read",
            json!({ "uri": "keep://castle/plan/top" }),
        );
        let contents = &plan["result"]["contents"][0];
        assert_eq!(contents["mimeType"], "image/svg+xml");
        assert!(contents["text"].as_str().unwrap().starts_with("<svg "));

        mock.finish();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].contains("\"method\":\"get_castle\""));
        assert!(requests[1].contains("\"method\":\"render_plan\""));
        assert!(requests[1].contains("top_plan"));
    }

    #[test]
    fn live_save_resource_is_rejected_as_offline_only() {
        let mut server =
            McpServer::live("http://127.0.0.1:1", "token", AgentPermission::DirectEdit).unwrap();
        let response = call(
            &mut server,
            22,
            "resources/read",
            json!({ "uri": "keep://castle/save" }),
        );
        let message = response["error"]["message"].as_str().unwrap();
        assert!(message.contains("--offline mode"), "got {message}");
    }

    #[test]
    fn live_mode_reports_http_status_before_parsing_the_body() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind status mock");
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept status request");
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            stream
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Type: text/plain\r\nContent-Length: 10\r\nConnection: close\r\n\r\nbad token\n",
                )
                .unwrap();
            stream.flush().unwrap();
        });

        let mut server = McpServer::live(
            &format!("http://127.0.0.1:{port}"),
            "token",
            AgentPermission::DirectEdit,
        )
        .unwrap();
        let message = tool_error_message(&mut server, "get_castle", json!({}));
        assert!(message.contains("HTTP 401"), "got {message}");
        assert!(message.contains("unauthorized"), "got {message}");
        assert!(
            !message.contains("valid JSON-RPC"),
            "status must be reported before JSON parsing: {message}"
        );
        handle.join().expect("status mock thread");
    }
}
