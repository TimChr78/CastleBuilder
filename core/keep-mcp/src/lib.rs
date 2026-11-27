//! Offline MCP sidecar for Keepwright.
//!
//! This crate links `keep-core` directly (architecture section 7.2, offline
//! mode): no game process, no GPU and no async runtime. It speaks line-delimited
//! JSON-RPC 2.0 over stdio, which makes it deterministic, cheap to spawn and
//! trivial to replay in tests.
//!
//! The protocol handler is a plain library type ([`McpServer`]) so every tool,
//! error case and atomic rollback can be unit tested without a subprocess. The
//! `keep-mcp` binary is a small stdin/stdout loop around it.

use std::collections::BTreeMap;

use keep_core::document::{
    BatteredWall, CastleDocument, Element, Opening, OpeningKind, RoundTower,
};
use keep_core::mesh::build_scene;
use keep_core::plan_render::{render_plan_svg, PlanView};
use keep_core::terrain::SiteTerrain;
use keep_proto::{
    validate_batch, validate_element_budget, ApplyOperationsRequest, ApplyOperationsResponse,
    ElementListResponse, ElementSummary, GetCastleResponse, OpeningSummary, Operation,
    RenderPlanRequest, RenderPlanResponse,
};
use serde::Serialize;
use serde_json::{json, Value};

/// MCP protocol revision implemented by this sidecar.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Server name reported during `initialize`.
pub const SERVER_NAME: &str = "keep-mcp";

/// JSON-RPC error codes used by the handler.
mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const RESOURCE_NOT_FOUND: i64 = -32002;
}

/// The offline MCP server: one castle document plus fixed terrain authority.
#[derive(Debug)]
pub struct McpServer {
    document: CastleDocument,
    terrain: SiteTerrain,
    /// Next id to hand out. Monotonic for the lifetime of the server so ids of
    /// deleted elements are never reused, which keeps earlier tool output
    /// unambiguous across turns.
    next_id: u64,
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new(false)
    }
}

impl McpServer {
    /// Create a server. `empty` starts from an empty document instead of the
    /// M0b showcase.
    pub fn new(empty: bool) -> Self {
        let document = if empty {
            CastleDocument::new()
        } else {
            keep_core::fixtures::m0b_showcase_document()
        };
        Self {
            next_id: max_existing_id(&document) + 1,
            document,
            terrain: SiteTerrain::m0a_promontory(),
        }
    }

    /// Current castle document (tests and embedders).
    pub fn document(&self) -> &CastleDocument {
        &self.document
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

        let result = match name {
            "get_castle" => self.tool_get_castle().map(|value| tool_result(&value)),
            "list_elements" => self.tool_list_elements().map(|value| tool_result(&value)),
            "render_plan" => self
                .tool_render_plan(arguments)
                .map(|value| tool_result(&value)),
            "apply_operations" => self
                .tool_apply_operations(arguments)
                .map(|value| tool_result(&value)),
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

    fn tool_get_castle(&self) -> Result<GetCastleResponse, String> {
        let scene = build_scene(&self.document, &self.terrain);
        Ok(GetCastleResponse {
            document_hash: hex_hash(self.document.document_hash()),
            layout_hash: hex_hash(scene.layout_hash),
            element_count: self.document.len(),
            opening_count: self.document.opening_count(),
            block_count: scene.block_count,
            chunk_count: scene.chunk_count(),
            elements: element_summaries(&self.document),
            openings: opening_summaries(&self.document),
        })
    }

    fn tool_list_elements(&self) -> Result<ElementListResponse, String> {
        Ok(ElementListResponse {
            element_count: self.document.len(),
            opening_count: self.document.opening_count(),
            elements: element_summaries(&self.document),
            openings: opening_summaries(&self.document),
        })
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
        let svg = render_plan_svg(&self.document, &self.terrain, view);
        Ok(RenderPlanResponse {
            view: view_name.to_string(),
            document_hash: hex_hash(self.document.document_hash()),
            mime_type: "image/svg+xml".to_string(),
            svg,
        })
    }

    /// Apply a batch atomically: all operations run against a clone first and
    /// the live document is only replaced when the whole batch succeeds.
    fn tool_apply_operations(
        &mut self,
        arguments: Value,
    ) -> Result<ApplyOperationsResponse, String> {
        let request: ApplyOperationsRequest = serde_json::from_value(arguments)
            .map_err(|error| format!("invalid arguments: {error}"))?;
        validate_batch(&request.operations).map_err(|error| error.to_string())?;

        let mut working = self.document.clone();
        let mut created_ids: BTreeMap<String, u64> = BTreeMap::new();
        let mut created_elements = 0usize;
        let mut created_openings = 0usize;
        let mut deleted = 0usize;
        let mut next_id = self.next_id;

        for operation in &request.operations {
            let operation = operation.validated().map_err(|error| error.to_string())?;
            match operation {
                Operation::CreateWallRun {
                    ref_id,
                    name,
                    start_mm,
                    end_mm,
                    height_mm,
                    thickness_mm,
                    batter_mm,
                } => {
                    let mut wall =
                        BatteredWall::new(next_id, name, start_mm, end_mm, height_mm, thickness_mm);
                    if let Some(batter_mm) = batter_mm {
                        wall.batter_mm = batter_mm;
                    }
                    working.add_wall(wall);
                    register_ref(&mut created_ids, ref_id, next_id)?;
                    next_id += 1;
                    created_elements += 1;
                }
                Operation::CreateTower {
                    ref_id,
                    name,
                    centre_mm,
                    radius_mm,
                    height_mm,
                    batter_mm,
                } => {
                    let mut tower = RoundTower::new(next_id, name, centre_mm, radius_mm, height_mm);
                    if let Some(batter_mm) = batter_mm {
                        tower.batter_mm = batter_mm;
                    }
                    working.add_tower(tower);
                    register_ref(&mut created_ids, ref_id, next_id)?;
                    next_id += 1;
                    created_elements += 1;
                }
                Operation::AddArrowLoop {
                    wall_id,
                    wall_ref,
                    t_permille,
                    sill_height_mm,
                    height_mm,
                    outer_slit_mm,
                    inner_splay_width_mm,
                } => {
                    let wall_id = match (wall_id, wall_ref.as_deref()) {
                        (Some(id), _) => id,
                        (None, Some(reference)) => *created_ids.get(reference).ok_or_else(|| {
                            format!("unknown wall_ref '{reference}' (declare it earlier in the batch)")
                        })?,
                        (None, None) => {
                            return Err("add_arrow_loop needs wall_id or wall_ref".into())
                        }
                    };
                    let wall = match working.get(wall_id) {
                        Some(Element::BatteredWall(wall)) => wall.clone(),
                        Some(_) => return Err(format!("element {wall_id} is not a wall run")),
                        None => return Err(format!("unknown wall_id {wall_id}")),
                    };
                    let opening = Opening::new(
                        next_id,
                        wall_id,
                        format!("Arrow loop {next_id}"),
                        OpeningKind::ArrowLoop,
                        t_permille,
                        sill_height_mm,
                        height_mm.unwrap_or(1_500),
                        outer_slit_mm.unwrap_or(140),
                        inner_splay_width_mm.unwrap_or(1_100),
                        wall.thickness_mm,
                    );
                    working.add_opening(opening);
                    next_id += 1;
                    created_openings += 1;
                }
                Operation::DeleteElement { id } => {
                    if !delete_id(&mut working, id) {
                        return Err(format!("unknown element id {id}"));
                    }
                    created_ids.retain(|_, value| *value != id);
                    deleted += 1;
                }
            }
        }

        // Cap the document before building the scene so a long-running agent
        // cannot exhaust sidecar memory with unbounded batches.
        validate_element_budget(working.len(), working.opening_count())
            .map_err(|error| error.to_string())?;

        let scene = build_scene(&working, &self.terrain);
        let summary = format!(
            "Applied {} operation(s): +{} element(s), +{} opening(s), -{}; {} block(s) in {} chunk(s)",
            request.operations.len(),
            created_elements,
            created_openings,
            deleted,
            scene.block_count,
            scene.chunk_count()
        );

        self.document = working;
        self.next_id = next_id;
        Ok(ApplyOperationsResponse {
            applied_count: request.operations.len(),
            created_ids,
            document_hash: hex_hash(self.document.document_hash()),
            layout_hash: hex_hash(scene.layout_hash),
            block_count: scene.block_count,
            chunk_count: scene.chunk_count(),
            summary,
        })
    }

    fn handle_resource_read(&self, params: &Value) -> Result<Value, (i64, String)> {
        let Some(uri) = params.get("uri").and_then(Value::as_str) else {
            return Err((code::INVALID_PARAMS, "missing resource uri".into()));
        };
        match uri {
            "keep://castle/document" => {
                let castle = self
                    .tool_get_castle()
                    .map_err(|message| (code::INVALID_PARAMS, message))?;
                let text = serde_json::to_string_pretty(&castle)
                    .map_err(|error| (code::INVALID_PARAMS, error.to_string()))?;
                Ok(resource_contents(uri, "application/json", text))
            }
            "keep://castle/plan/top" => Ok(self.resource_svg(uri, PlanView::TopPlan)),
            "keep://castle/plan/elevation" => Ok(self.resource_svg(uri, PlanView::ElevationNorth)),
            other => Err((
                code::RESOURCE_NOT_FOUND,
                format!("unknown resource: {other}"),
            )),
        }
    }

    fn resource_svg(&self, uri: &str, view: PlanView) -> Value {
        let svg = render_plan_svg(&self.document, &self.terrain, view);
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
        "instructions": "Offline Keepwright castle sidecar. Use get_castle, list_elements, render_plan and apply_operations."
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

fn element_summaries(document: &CastleDocument) -> Vec<ElementSummary> {
    document
        .elements()
        .map(|element| match element {
            Element::BatteredWall(wall) => ElementSummary {
                id: wall.id,
                kind: "wall".to_string(),
                name: wall.name.clone(),
                height_mm: wall.height_mm,
                thickness_mm: wall.thickness_mm,
                start_mm: Some(wall.start_mm),
                end_mm: Some(wall.end_mm),
                centre_mm: None,
                radius_mm: None,
                length_mm: Some(wall.length_mm()),
            },
            Element::RoundTower(tower) => ElementSummary {
                id: tower.id,
                kind: "tower".to_string(),
                name: tower.name.clone(),
                height_mm: tower.height_mm,
                thickness_mm: tower.thickness_mm,
                start_mm: None,
                end_mm: None,
                centre_mm: Some(tower.centre_mm),
                radius_mm: Some(tower.radius_mm),
                length_mm: None,
            },
        })
        .collect()
}

fn opening_summaries(document: &CastleDocument) -> Vec<OpeningSummary> {
    document
        .openings()
        .map(|opening| OpeningSummary {
            id: opening.id,
            wall_id: opening.wall_id,
            kind: "arrow_loop".to_string(),
            name: opening.name.clone(),
            t_permille: opening.t_permille,
            sill_height_mm: opening.sill_height_mm,
            height_mm: opening.height_mm,
            outer_slit_mm: opening.outer_slit_mm,
            inner_splay_width_mm: opening.inner_splay_width_mm,
        })
        .collect()
}

fn register_ref(
    created_ids: &mut BTreeMap<String, u64>,
    ref_id: Option<String>,
    id: u64,
) -> Result<(), String> {
    if let Some(reference) = ref_id {
        keep_proto::sanitize_ref_id(&reference).map_err(|error| error.to_string())?;
        if created_ids.insert(reference.clone(), id).is_some() {
            return Err(format!("duplicate ref_id '{reference}' in batch"));
        }
    }
    Ok(())
}

fn max_existing_id(document: &CastleDocument) -> u64 {
    let max_element = document.elements().map(Element::id).max().unwrap_or(0);
    let max_opening = document
        .openings()
        .map(|opening| opening.id)
        .max()
        .unwrap_or(0);
    max_element.max(max_opening)
}

/// Delete an element (with its attached openings) or a lone opening. Returns
/// `false` when the id is unknown so the caller can fail the batch.
fn delete_id(document: &mut CastleDocument, id: u64) -> bool {
    let element_exists = document.get(id).is_some();
    let opening_exists = document.opening(id).is_some();
    if !element_exists && !opening_exists {
        return false;
    }
    let elements: Vec<Element> = document
        .elements()
        .filter(|element| element.id() != id)
        .cloned()
        .collect();
    let openings: Vec<Opening> = document
        .openings()
        .filter(|opening| opening.id != id && opening.wall_id != id)
        .cloned()
        .collect();
    let mut next = CastleDocument::new();
    for element in elements {
        next.insert(element);
    }
    for opening in openings {
        next.insert_opening(opening);
    }
    *document = next;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use keep_proto::{MAX_BATCH_OPS, MAX_NAME_CHARS};

    fn server() -> McpServer {
        McpServer::new(false)
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
    fn tools_list_exposes_four_tools_with_schemas() {
        let mut server = server();
        let response = call(&mut server, 4, "tools/list", json!({}));
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 4);
        for tool in tools {
            assert!(tool["inputSchema"].is_object());
            assert!(tool["outputSchema"].is_object());
            assert!(tool["annotations"].is_object());
        }
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
        assert_eq!(resources.len(), 3);
        assert_eq!(resources[0]["uri"], "keep://castle/document");
        assert_eq!(resources[1]["mimeType"], "image/svg+xml");

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
    }

    #[test]
    fn unknown_resource_is_an_error() {
        let mut server = server();
        let response = call(
            &mut server,
            8,
            "resources/read",
            json!({ "uri": "keep://castle/assessment" }),
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
}
