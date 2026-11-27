//! Thin GDExtension binding: commands in, mesh buffers out.
//!
//! `KeepBridge` is a thin *session* bridge, not an authority: it forwards
//! interactive edits to a [`keep_core::session::Session`], which owns the
//! authoritative [`keep_core::document::CastleDocument`], the append-only
//! command log and the undo semantics. Every rebuild still runs the
//! deterministic `keep-core` pipeline. The returned dictionary is Godot
//! scenery, never the source of truth.

mod control_server;

use std::sync::mpsc::Sender;
use std::time::Instant;

use godot::builtin::{
    Array, Color, GString, PackedColorArray, PackedFloat32Array, PackedInt32Array,
    PackedVector2Array, PackedVector3Array, VarArray, VarDictionary, Vector2, Vector3, Vector3i,
};
use godot::classes::mesh::{ArrayCustomFormat, ArrayFormat, ArrayType, PrimitiveType};
use godot::classes::{ArrayMesh, INode, Node};
use godot::obj::{EngineBitfield, EngineEnum};
use godot::prelude::*;
use serde_json::{json, Value};

use control_server::{ControlServer, PendingRequest, RpcError, RpcReply};
use keep_core::command::{
    AuthorId, CommandOp, MAX_TOWER_HEIGHT_MM, MAX_TOWER_RADIUS_MM, MAX_WALL_HEIGHT_MM,
    MAX_WALL_THICKNESS_MM, MIN_TOWER_HEIGHT_MM, MIN_TOWER_RADIUS_MM, MIN_WALL_HEIGHT_MM,
    MIN_WALL_THICKNESS_MM,
};
use keep_core::document::{CastleDocument, Element, RangeKind, TowerPlan};
use keep_core::historian::{assess_historian, HistorianCategory, HistorianReport};
use keep_core::mesh::{ChunkMeshData, SceneData};
use keep_core::picking::SnapKind;
use keep_core::plan_render::{render_plan_svg, PlanView};
use keep_core::proposal::{
    AgentParticipant, AgentPermission, AgentState, Proposal, ProposalStatus,
};
use keep_core::session::Session;
use keep_core::style::StylePack;
use keep_core::terrain::{SiteTerrain, TerrainMeshData};
use keep_core::validate::{validate_structure, Severity};
use keep_proto::{
    build_castle_response, build_element_list_response, RenderViewResponse, DEFAULT_PLAN_VIEW,
};

/// Site the M1 vertical slice starts on.
const M1_SITE_ID: &str = "coastal_headland";

/// The single fixed participant that every authenticated HTTP control request
/// acts as. Request parameters can never select or register another agent.
const CONTROL_AGENT_ID: &str = "mcp-agent";
/// Agent id that authors the in-game demo proposal.
const DEMO_AGENT_ID: &str = "mason";

/// What executing one control request produced.
enum ControlOutcome {
    Reply(Value),
    RenderView(i64, keep_proto::RenderViewRequest),
}

/// The only class the game needs for M0a/M0b.
#[derive(GodotClass)]
#[class(base = Node)]
pub struct KeepBridge {
    base: Base<Node>,
    /// Host-authoritative session: document, command log and undo live here.
    session: Session,
    /// Author used for every local interactive edit.
    player_author: AuthorId,
    /// Running localhost JSON-RPC control server, if started.
    control_server: Option<ControlServer>,
    /// A `render_view` capture that `main.gd` still has to fulfill.
    pending_render_view: Option<(i64, Sender<RpcReply>)>,
    /// Monotonic id for `render_view` capture requests.
    next_render_view_id: i64,
    /// Active architectural style pack id (`edwardian_welsh` by default).
    style_id: String,
}

#[godot_api]
impl INode for KeepBridge {
    fn init(base: Base<Node>) -> Self {
        let mut bridge = Self {
            base,
            session: Session::from_document(
                M1_SITE_ID,
                SiteTerrain::for_site(M1_SITE_ID).expect("coastal_headland is a curated site"),
                keep_core::fixtures::m1_edwardian_document(),
            ),
            player_author: AuthorId::player("local")
                .expect("built-in local player author id is always valid"),
            control_server: None,
            pending_render_view: None,
            next_render_view_id: 1,
            style_id: "edwardian_welsh".to_string(),
        };
        bridge.ensure_control_agents();
        bridge
    }
}

#[godot_api]
impl KeepBridge {
    /// Build the M1 Edwardian showcase on the coastal headland.
    #[func]
    fn build_m1_edwardian(&mut self) -> VarDictionary {
        self.style_id = "edwardian_welsh".to_string();
        self.replace_session(Session::from_document(
            M1_SITE_ID,
            SiteTerrain::for_site(M1_SITE_ID).expect("coastal_headland is a curated site"),
            keep_core::fixtures::m1_edwardian_document(),
        ));
        self.session_scene()
    }

    /// Build the M2 Baltic red-brick showcase on the lake island.
    #[func]
    fn build_m2_baltic(&mut self) -> VarDictionary {
        self.style_id = "baltic_brick".to_string();
        self.replace_session(Session::from_document(
            "lake_island",
            SiteTerrain::for_site("lake_island").expect("lake_island is a curated site"),
            keep_core::fixtures::m2_baltic_document(),
        ));
        self.session_scene()
    }

    /// Switch the active architectural style and re-assess the live document.
    #[func]
    fn set_style(&mut self, style_id: GString) -> VarDictionary {
        let id = style_id.to_string();
        let Some(pack) = StylePack::find(&id) else {
            let mut result = VarDictionary::new();
            result.set("ok", false);
            result.set("error", &GString::from(&format!("unknown style id '{id}'")));
            return result;
        };
        self.style_id = pack.id.clone();
        let historian = assess_historian(self.session.document(), self.session.terrain(), &pack);
        let mut result = VarDictionary::new();
        result.set("ok", true);
        result.set("style_id", &GString::from(&pack.id));
        result.set("material_palette", &GString::from(&pack.material_palette));
        result.set(
            "assessment",
            &assessment_dictionary(self.session.document(), self.session.terrain(), &pack),
        );
        result.set("historian", &historian_dictionary(&historian));
        result
    }

    /// Switch to an empty castle on one of the curated sites.
    #[func]
    fn load_site(&mut self, site_id: GString) -> VarDictionary {
        let site = site_id.to_string();
        let Some(terrain) = SiteTerrain::for_site(&site) else {
            let mut result = VarDictionary::new();
            result.set("ok", false);
            result.set(
                "error",
                &GString::from(&format!("unknown site id '{site}'")),
            );
            return result;
        };
        self.replace_session(Session::from_document(site, terrain, CastleDocument::new()));
        let mut result = self.session_scene();
        result.set("ok", true);
        result
    }

    /// Describe every curated site.
    #[func]
    fn list_sites(&self) -> Array<VarDictionary> {
        let mut sites: Array<VarDictionary> = Array::new();
        for info in SiteTerrain::list_sites() {
            let mut entry = VarDictionary::new();
            entry.set("id", &GString::from(&info.id));
            entry.set("name", &GString::from(&info.name));
            entry.set("description", &GString::from(&info.description));
            sites.push(&entry);
        }
        sites
    }

    /// Build the fixed M0a look scene (one battered wall, one round tower).
    #[func]
    fn build_m0a_showcase(&mut self) -> VarDictionary {
        self.replace_session(Session::from_document(
            "m0a_promontory",
            SiteTerrain::m0a_promontory(),
            keep_core::fixtures::m0a_showcase_document(),
        ));
        self.session_scene()
    }

    /// Build the M0b junction and void look scene.
    #[func]
    fn build_m0b_showcase(&mut self) -> VarDictionary {
        self.replace_session(Session::from_document(
            "m0a_promontory",
            SiteTerrain::m0a_promontory(),
            keep_core::fixtures::m0b_showcase_document(),
        ));
        self.session_scene()
    }

    /// Build the large concentric-castle stress scene (~200k blocks).
    #[func]
    fn build_stress_200k(&mut self) -> VarDictionary {
        self.replace_session(Session::from_document(
            "m0a_promontory",
            SiteTerrain::m0a_promontory(),
            keep_core::fixtures::stress_200k_document(),
        ));
        self.session_scene()
    }

    /// Pick the promontory ground under a world-space ray. `point` is in metres.
    #[func]
    fn pick_terrain(&self, ray_origin: Vector3, ray_dir: Vector3) -> VarDictionary {
        let hit = keep_core::picking::pick_terrain(
            self.session.terrain(),
            [ray_origin.x, ray_origin.y, ray_origin.z],
            [ray_dir.x, ray_dir.y, ray_dir.z],
        );

        let mut result = VarDictionary::new();
        match hit {
            Some([x_mm, y_mm, z_mm]) => {
                result.set("hit", true);
                result.set(
                    "point",
                    Vector3::new(mm_to_m(x_mm), mm_to_m(y_mm), mm_to_m(z_mm)),
                );
            }
            None => {
                result.set("hit", false);
                result.set("point", Vector3::ZERO);
            }
        }
        result
    }

    /// Snap a plan point to nearby tower centres/perimeters or wall endpoints.
    /// `point_xz` and the returned `point` are in metres.
    #[func]
    fn snap_point(&self, point_xz: Vector2, snap_radius_m: f32) -> VarDictionary {
        let point_mm = quantise_xz(point_xz);
        let radius_mm = (f64::from(snap_radius_m.max(0.0)) * 1000.0).round() as i64;
        let snap = keep_core::picking::snap_point(self.session.document(), point_mm, radius_mm);

        let mut result = VarDictionary::new();
        result.set("snapped", snap.is_snapped());
        result.set(
            "point",
            Vector2::new(mm_to_m(snap.point_mm[0]), mm_to_m(snap.point_mm[1])),
        );
        result.set("label", &snap_label(snap.kind));
        result.set("kind", &snap_kind_key(snap.kind));
        result.set(
            "element_id",
            snap.element_id.map(|id| id as i64).unwrap_or(-1),
        );
        result
    }

    /// Add a battered curtain wall and return the rebuilt scene.
    ///
    /// The edit is a normal command batch, so it is validated, logged and
    /// undoable exactly like an agent edit.
    #[func]
    fn add_wall(
        &mut self,
        start_xz: Vector2,
        end_xz: Vector2,
        height_m: f32,
        thickness_m: f32,
    ) -> VarDictionary {
        let height_mm = quantise_m(height_m).clamp(MIN_WALL_HEIGHT_MM, MAX_WALL_HEIGHT_MM);
        let thickness_mm =
            quantise_m(thickness_m).clamp(MIN_WALL_THICKNESS_MM, MAX_WALL_THICKNESS_MM);
        let op = CommandOp::CreateWallRun {
            ref_id: None,
            name: format!("Wall {}", self.session.next_element_id()),
            start_mm: quantise_xz(start_xz),
            end_mm: quantise_xz(end_xz),
            height_mm,
            thickness_mm,
            batter_mm: None,
        };
        match self
            .session
            .apply_batch(self.player_author.clone(), vec![op])
        {
            Ok(_) => {
                let mut result = self.session_scene();
                result.set("ok", true);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Add a round tower and return the rebuilt scene.
    #[func]
    fn add_tower(&mut self, center_xz: Vector2, radius_m: f32, height_m: f32) -> VarDictionary {
        let radius_mm = quantise_m(radius_m).clamp(MIN_TOWER_RADIUS_MM, MAX_TOWER_RADIUS_MM);
        let height_mm = quantise_m(height_m).clamp(MIN_TOWER_HEIGHT_MM, MAX_TOWER_HEIGHT_MM);
        let op = CommandOp::CreateTower {
            ref_id: None,
            name: format!("Tower {}", self.session.next_element_id()),
            centre_mm: quantise_xz(center_xz),
            radius_mm,
            height_mm,
            batter_mm: None,
        };
        match self
            .session
            .apply_batch(self.player_author.clone(), vec![op])
        {
            Ok(_) => {
                let mut result = self.session_scene();
                result.set("ok", true);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Add a tower with a round, square or D-shaped plan.
    ///
    /// A shaped plan needs a `ModifyElement` follow-up, which takes an existing
    /// `ElementId`. Because ids are allocated deterministically from the
    /// session's `next_element_id`, the create is the first operation in the
    /// batch and therefore receives exactly that id, so both operations stay in
    /// one atomic batch (the session validates and applies all-or-nothing).
    #[func]
    fn add_tower_shaped(
        &mut self,
        center_xz: Vector2,
        radius_m: f32,
        height_m: f32,
        plan: GString,
        orientation_deg: i32,
    ) -> VarDictionary {
        let radius_mm = quantise_m(radius_m).clamp(MIN_TOWER_RADIUS_MM, MAX_TOWER_RADIUS_MM);
        let height_mm = quantise_m(height_m).clamp(MIN_TOWER_HEIGHT_MM, MAX_TOWER_HEIGHT_MM);
        let tower_plan = tower_plan_from_key(&plan.to_string());
        let created_id = self.session.next_element_id();
        let mut ops = vec![CommandOp::CreateTower {
            ref_id: None,
            name: format!("Tower {created_id}"),
            centre_mm: quantise_xz(center_xz),
            radius_mm,
            height_mm,
            batter_mm: None,
        }];
        if tower_plan != TowerPlan::Round {
            ops.push(CommandOp::ModifyElement {
                id: created_id,
                height_mm: None,
                thickness_mm: None,
                batter_mm: None,
                plinth_height_mm: None,
                plan: Some(tower_plan),
                orientation_deg: Some(orientation_deg),
            });
        }
        match self.session.apply_batch(self.player_author.clone(), ops) {
            Ok(_) => {
                let mut result = self.session_scene();
                result.set("ok", true);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Add a great rectangular keep and return the rebuilt scene.
    #[func]
    fn add_keep(&mut self, center_xz: Vector2, orientation_deg: i32) -> VarDictionary {
        let op = CommandOp::CreateKeep {
            ref_id: None,
            name: format!("Keep {}", self.session.next_element_id()),
            centre_mm: quantise_xz(center_xz),
            width_mm: None,
            depth_mm: None,
            height_mm: None,
            wall_thickness_mm: None,
            orientation_deg: Some(orientation_deg),
            batter_mm: None,
            plinth_height_mm: None,
        };
        match self
            .session
            .apply_batch(self.player_author.clone(), vec![op])
        {
            Ok(_) => {
                let mut result = self.session_scene();
                result.set("ok", true);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Add a roofed domestic range (`hall`, `chapel` or `kitchen`).
    #[func]
    fn add_range(
        &mut self,
        center_xz: Vector2,
        kind: GString,
        orientation_deg: i32,
    ) -> VarDictionary {
        let range_kind = range_kind_from_key(&kind.to_string());
        let op = CommandOp::CreateRange {
            ref_id: None,
            name: format!(
                "{} {}",
                range_kind_name(range_kind),
                self.session.next_element_id()
            ),
            kind: range_kind,
            centre_mm: quantise_xz(center_xz),
            length_mm: None,
            width_mm: None,
            wall_height_mm: None,
            roof_pitch_deg: None,
            orientation_deg: Some(orientation_deg),
        };
        match self
            .session
            .apply_batch(self.player_author.clone(), vec![op])
        {
            Ok(_) => {
                let mut result = self.session_scene();
                result.set("ok", true);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Add a twin-towered gatehouse and return the rebuilt scene.
    #[func]
    fn add_gatehouse(&mut self, center_xz: Vector2, orientation_deg: i32) -> VarDictionary {
        let op = CommandOp::CreateGatehouse {
            ref_id: None,
            name: format!("Gatehouse {}", self.session.next_element_id()),
            centre_mm: quantise_xz(center_xz),
            orientation_deg: Some(orientation_deg),
            passage_width_mm: None,
            height_mm: None,
            tower_radius_mm: None,
        };
        match self
            .session
            .apply_batch(self.player_author.clone(), vec![op])
        {
            Ok(_) => {
                let mut result = self.session_scene();
                result.set("ok", true);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Carve a linear ditch and return the rebuilt scene.
    #[func]
    fn add_ditch(
        &mut self,
        start_xz: Vector2,
        end_xz: Vector2,
        width_m: f32,
        depth_m: f32,
    ) -> VarDictionary {
        let op = CommandOp::CreateDitch {
            ref_id: None,
            name: format!("Ditch {}", self.session.next_element_id()),
            start_mm: quantise_xz(start_xz),
            end_mm: quantise_xz(end_xz),
            width_mm: quantise_m(width_m),
            depth_mm: quantise_m(depth_m),
        };
        match self
            .session
            .apply_batch(self.player_author.clone(), vec![op])
        {
            Ok(_) => {
                let mut result = self.session_scene();
                result.set("ok", true);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Assess the current castle against the active style pack.
    #[func]
    fn get_assessment(&self) -> VarDictionary {
        let style = self.active_style();
        assessment_dictionary(self.session.document(), self.session.terrain(), &style)
    }

    /// Apply a suggested fix (serialised `CommandOp` JSON) as a normal batch.
    #[func]
    fn apply_fix_json(&mut self, fix_json: GString) -> VarDictionary {
        let op = match command_op_from_json(&fix_json.to_string()) {
            Ok(op) => op,
            Err(message) => {
                let mut result = VarDictionary::new();
                result.set("ok", false);
                result.set("error", &GString::from(&message));
                return result;
            }
        };
        match self
            .session
            .apply_batch(self.player_author.clone(), vec![op])
        {
            Ok(_) => {
                let mut result = self.session_scene();
                result.set("ok", true);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Compensate the most recent local batch. History is append-only: the undo
    /// is itself a command and never truncates the log.
    #[func]
    fn undo_last(&mut self) -> VarDictionary {
        match self.session.undo_last(&self.player_author) {
            Ok(undo) => {
                let mut result = self.session_scene();
                result.set("undone", true);
                result.set("ok", true);
                result.set("target_seq", undo.target_seq as i64);
                result.set("summary", &GString::from(&undo.summary));
                result
            }
            Err(error) => {
                // Nothing changed, so skip the expensive scene rebuild and
                // report the refusal directly to the UI.
                let mut result = VarDictionary::new();
                let message = GString::from(&error.to_string());
                result.set("undone", false);
                result.set("ok", false);
                result.set("reason", &message);
                result.set("error", &message);
                result
            }
        }
    }

    /// Serialise the session as `castle.keep.json` text.
    #[func]
    fn save_castle_json(&self) -> VarDictionary {
        match self.session.to_save_json() {
            Ok(json) => {
                let mut result = VarDictionary::new();
                result.set("ok", true);
                result.set("json", &GString::from(&json));
                result.set(
                    "document_hash",
                    &GString::from(&format!("{:016x}", self.session.document().document_hash())),
                );
                result.set("history_len", self.session.history().len() as i64);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Load a `castle.keep.json` text, replaying and verifying its command log.
    #[func]
    fn load_castle_json(&mut self, json: GString) -> VarDictionary {
        match Session::from_save_json(&json.to_string()) {
            Ok(session) => {
                self.replace_session(session);
                let mut result = self.session_scene();
                result.set("ok", true);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    // -----------------------------------------------------------------------
    // M1.3: localhost control server, proposals and the in-game agent panel
    // -----------------------------------------------------------------------

    /// Start the loopback JSON-RPC control server (architecture sections 7.2
    /// and 7.6). `preferred_port` is tried first; a busy port falls back to an
    /// ephemeral one. Returns the URL, actual port and per-session token.
    #[func]
    fn start_control_server(&mut self, preferred_port: i32) -> VarDictionary {
        self.control_server = None;
        self.pending_render_view = None;
        self.ensure_control_agents();

        let mut result = VarDictionary::new();
        match ControlServer::start(preferred_port) {
            Ok(server) => {
                let url = server.url();
                let port = i64::from(server.port);
                let token = server.token.clone();
                self.control_server = Some(server);
                result.set("ok", true);
                result.set("url", &GString::from(&url));
                result.set("port", port);
                result.set("token", &GString::from(&token));
                result.set("error", &GString::from(""));
            }
            Err(error) => {
                result.set("ok", false);
                result.set("url", &GString::from(""));
                result.set("port", 0i64);
                result.set("token", &GString::from(""));
                result.set("error", &GString::from(&error));
            }
        }
        result
    }

    /// Drain queued control requests and execute them on the main thread.
    ///
    /// The listener thread never touches Godot: it only forwards the parsed
    /// JSON-RPC envelope here. The returned flags tell `main.gd` what visually
    /// changed, and `render_view_request_id` asks it to capture a PNG.
    #[func]
    fn poll_control(&mut self) -> VarDictionary {
        let mut dirty = false;
        let mut proposals_changed = false;
        let mut render_view_request_id: i64 = -1;
        let mut render_view_options: Option<VarDictionary> = None;

        loop {
            let Some(request) = self
                .control_server
                .as_ref()
                .and_then(ControlServer::drain_live)
            else {
                break;
            };
            let PendingRequest {
                method,
                params,
                reply,
                deadline,
            } = request;
            if Instant::now() >= deadline {
                // The HTTP worker has already given up on this request. Drop it
                // without executing it so a stalled client cannot mutate state
                // after its deadline.
                let _ = reply.send(RpcReply::Err(RpcError::new(
                    -32_000,
                    "request expired before it reached the game thread",
                )));
                continue;
            }

            let before_hash = self.session.document().document_hash();
            let before_proposals = proposal_signature(&self.session);
            let outcome = self.execute_control(&method, &params);
            if self.session.document().document_hash() != before_hash {
                dirty = true;
            }
            if proposal_signature(&self.session) != before_proposals {
                proposals_changed = true;
            }

            match outcome {
                Ok(ControlOutcome::Reply(value)) => {
                    let _ = reply.send(RpcReply::Ok(value));
                }
                Ok(ControlOutcome::RenderView(request_id, view_req)) => {
                    if self.pending_render_view.is_some() {
                        let _ = reply.send(RpcReply::Err(RpcError::new(
                            -32_000,
                            "a render_view capture is already pending",
                        )));
                    } else {
                        self.pending_render_view = Some((request_id, reply));
                        render_view_request_id = request_id;
                        let mut opts = VarDictionary::new();
                        if let Some(yaw) = view_req.yaw_deg {
                            opts.set("yaw_deg", f64::from(yaw));
                        }
                        if let Some(pitch) = view_req.pitch_deg {
                            opts.set("pitch_deg", f64::from(pitch));
                        }
                        if let Some(dist) = view_req.distance_m {
                            opts.set("distance_m", f64::from(dist));
                        }
                        render_view_options = Some(opts);
                    }
                }
                Err(error) => {
                    let _ = reply.send(RpcReply::Err(error));
                }
            }
        }

        let mut result = VarDictionary::new();
        result.set("dirty", dirty);
        result.set("proposals_changed", proposals_changed);
        result.set("render_view_request_id", render_view_request_id);
        if let Some(opts) = render_view_options {
            result.set("render_view", &opts);
        }
        result
    }

    /// Fulfil a queued `render_view` request with a base64 PNG (768x512).
    #[func]
    fn complete_render_view(&mut self, request_id: i64, png_base64: GString, error: GString) {
        let Some((pending_id, reply)) = self.pending_render_view.take() else {
            return;
        };
        if pending_id != request_id {
            self.pending_render_view = Some((pending_id, reply));
            return;
        }
        let message = error.to_string();
        let outcome = if message.trim().is_empty() {
            let view = RenderViewResponse {
                width: 768,
                height: 512,
                mime_type: "image/png".to_string(),
                image_base64: png_base64.to_string(),
            };
            match serde_json::to_value(view) {
                Ok(value) => RpcReply::Ok(value),
                Err(error) => RpcReply::Err(RpcError::new(
                    -32_000,
                    format!("render_view could not be serialised: {error}"),
                )),
            }
        } else {
            RpcReply::Err(RpcError::new(
                -32_000,
                format!("render_view failed: {message}"),
            ))
        };
        let _ = reply.send(outcome);
    }

    /// Rebuild the current document on demand. `main.gd` calls this when
    /// `poll_control()` reports that an agent changed the castle.
    #[func]
    fn get_scene_snapshot(&self) -> VarDictionary {
        self.session_scene()
    }

    /// Everything the in-game agent panel needs (section 7.7).
    #[func]
    fn get_agent_panel_state(&self) -> VarDictionary {
        let mut result = VarDictionary::new();
        let url = self
            .control_server
            .as_ref()
            .map(ControlServer::url)
            .unwrap_or_default();
        result.set("server_url", &GString::from(&url));
        result.set("server_running", self.control_server.is_some());

        let mut agents: Array<VarDictionary> = Array::new();
        for agent in self.session.agents() {
            let mut entry = VarDictionary::new();
            entry.set("id", &GString::from(&agent.id));
            entry.set("name", &GString::from(&agent.name));
            entry.set("color", &GString::from(&agent.color_hex));
            entry.set(
                "permission",
                &GString::from(permission_key(agent.permission)),
            );
            entry.set("state", &GString::from(state_key(agent.state)));
            agents.push(&entry);
        }
        result.set("agents", &agents);

        let mut proposals: Array<VarDictionary> = Array::new();
        for proposal in self.session.proposals() {
            let mut entry = VarDictionary::new();
            entry.set("id", &GString::from(&proposal.id));
            entry.set("title", &GString::from(&proposal.title));
            entry.set("author", &GString::from(&proposal.author.to_string()));
            entry.set("summary", &GString::from(&proposal.summary));
            entry.set("semantic_diff", &GString::from(&proposal.semantic_diff));
            entry.set("structure_before", i64::from(proposal.structure_before));
            entry.set("structure_after", i64::from(proposal.structure_after));
            entry.set("structure_delta", i64::from(proposal.structure_delta));
            entry.set("status", &GString::from(status_key(proposal.status)));
            entry.set(
                "reject_reason",
                &GString::from(proposal.reject_reason.as_deref().unwrap_or("")),
            );
            entry.set("op_count", proposal.ops.len() as i64);
            proposals.push(&entry);
        }
        result.set("proposals", &proposals);
        result
    }

    /// Change one agent's permission from the panel toggle.
    #[func]
    fn set_agent_permission(&mut self, agent_id: GString, permission: GString) -> bool {
        let permission = permission_from_key(&permission.to_string());
        self.session
            .set_agent_permission(&agent_id.to_string(), permission)
            .is_ok()
    }

    /// Human approval: apply a submitted proposal as one atomic batch and
    /// return the rebuilt scene (architecture sections 4.2 and 7.7).
    #[func]
    fn accept_proposal(&mut self, proposal_id: GString) -> VarDictionary {
        match self.session.accept_proposal(&proposal_id.to_string()) {
            Ok(_) => {
                let mut result = self.session_scene();
                result.set("ok", true);
                result.set("proposal_id", &proposal_id);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Refuse a submitted proposal; the live document stays untouched.
    #[func]
    fn reject_proposal(&mut self, proposal_id: GString, reason: GString) -> VarDictionary {
        let reason = reason.to_string();
        let reason = if reason.trim().is_empty() {
            None
        } else {
            Some(reason.as_str())
        };
        match self
            .session
            .reject_proposal(&proposal_id.to_string(), reason)
        {
            Ok(_) => {
                let mut result = VarDictionary::new();
                result.set("ok", true);
                result.set("proposal_id", &proposal_id);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Build the translucent ghost scene for one proposal. Only the elements
    /// the proposal adds or modifies are included.
    #[func]
    fn build_proposal_ghost(&self, proposal_id: GString) -> VarDictionary {
        match self
            .session
            .build_proposal_ghost_scene(&proposal_id.to_string())
        {
            Some(scene) => {
                let style = self.active_style();
                let mut result = scene_to_dictionary(
                    scene,
                    self.session.document(),
                    self.session.terrain(),
                    self.session.document().document_hash(),
                    self.session.site_id(),
                    &style,
                );
                result.set("ok", true);
                result
            }
            None => {
                let mut result = VarDictionary::new();
                result.set("ok", false);
                result.set("chunks", &Array::<VarDictionary>::new());
                result.set(
                    "error",
                    &GString::from("proposal ghost could not be generated"),
                );
                result
            }
        }
    }

    /// Create and submit a realistic demo proposal (a flanking tower plus a
    /// curtain extension), so the panel's Accept/Reject flow is immediately
    /// testable in game.
    #[func]
    fn create_demo_proposal(&mut self) -> VarDictionary {
        self.ensure_control_agents();
        let author = match AuthorId::agent(DEMO_AGENT_ID) {
            Ok(author) => author,
            Err(error) => return error_dictionary(&error),
        };
        let proposal = match self
            .session
            .begin_proposal(author.clone(), "Flanking tower & curtain extension")
        {
            Ok(proposal) => proposal,
            Err(error) => return error_dictionary(&error),
        };
        let ops = vec![
            CommandOp::CreateWallRun {
                ref_id: None,
                name: "South curtain extension".to_string(),
                start_mm: [-32_000, 5_100],
                end_mm: [-43_000, 5_100],
                height_mm: 9_000,
                thickness_mm: 2_400,
                batter_mm: Some(900),
            },
            CommandOp::CreateTower {
                ref_id: None,
                name: "South-west flanking tower".to_string(),
                centre_mm: [-43_000, 5_100],
                radius_mm: 3_600,
                height_mm: 13_000,
                batter_mm: Some(600),
            },
        ];
        if let Err(error) = self.session.stage_proposal_ops(&proposal.id, &author, ops) {
            return error_dictionary(&error);
        }
        let submitted = match self.session.submit_proposal(
            &proposal.id,
            &author,
            "Adds a round flanking tower and extends the south curtain by 11 m.",
        ) {
            Ok(proposal) => proposal,
            Err(error) => return error_dictionary(&error),
        };

        let mut result = VarDictionary::new();
        result.set("ok", true);
        result.set("proposal_id", &GString::from(&submitted.id));
        result.set("title", &GString::from(&submitted.title));
        result.set("summary", &GString::from(&submitted.summary));
        result.set("semantic_diff", &GString::from(&submitted.semantic_diff));
        result.set("structure_before", i64::from(submitted.structure_before));
        result.set("structure_after", i64::from(submitted.structure_after));
        result
    }
}

impl KeepBridge {
    /// Register the built-in panel agents on a freshly built session. Sessions
    /// are replaced by every mode/site load, so presence is re-established here.
    fn ensure_control_agents(&mut self) {
        self.ensure_agent(CONTROL_AGENT_ID, "MCP Agent", "#4fa3ff");
        self.ensure_agent(DEMO_AGENT_ID, "Mason", "#e8b04b");
    }

    fn ensure_agent(&mut self, id: &str, name: &str, color_hex: &str) {
        if self.session.agent(id).is_none() {
            let _ = self
                .session
                .register_agent(id, name, color_hex, AgentPermission::ProposalOnly);
        }
    }

    /// Replace the authoritative session while carrying registered agent
    /// permissions and states across the swap (M1.3 M6). A session swap must
    /// never silently re-promote an agent the human demoted to read-only.
    fn replace_session(&mut self, session: Session) {
        let registered: Vec<AgentParticipant> = self.session.agents().cloned().collect();
        self.session = session;
        self.ensure_control_agents();
        for agent in registered {
            let _ = self.session.register_agent(
                &agent.id,
                &agent.name,
                &agent.color_hex,
                agent.permission,
            );
            self.session.set_agent_state(&agent.id, agent.state);
        }
    }

    /// Execute one control-protocol method on the authoritative session.
    fn execute_control(
        &mut self,
        method: &str,
        params: &Value,
    ) -> Result<ControlOutcome, RpcError> {
        // B3: every authenticated HTTP request acts as the same fixed
        // registered participant. `params.agent_id` is never consulted, so a
        // caller cannot impersonate a direct-edit agent or register one.
        let author = AuthorId::agent(CONTROL_AGENT_ID)
            .map_err(|error| RpcError::new(-32_602, format!("invalid agent id: {error}")))?;

        let value = match method {
            "get_castle" => serde_json::to_value(build_castle_response(&self.session))
                .map_err(response_error)?,
            "list_elements" => serde_json::to_value(build_element_list_response(&self.session))
                .map_err(response_error)?,
            "get_assessment" => {
                // Match the offline `keep-mcp` contract: an explicit style_id
                // is honoured (and rejected when unknown); only an omitted
                // style falls back to the live session's active pack.
                let style = match params.get("style_id") {
                    None | Some(Value::Null) => self.active_style(),
                    Some(Value::String(id)) => StylePack::find(id).ok_or_else(|| {
                        RpcError::new(-32_602, format!("unknown style id '{id}'"))
                    })?,
                    Some(_) => {
                        return Err(RpcError::new(-32_602, "'style_id' must be a string"));
                    }
                };
                assessment_json(self.session.document(), self.session.terrain(), &style)
            }
            "list_styles" => json!({ "styles": StylePack::list() }),
            "get_style" => {
                let id = required_string(params, "id")?;
                let pack = StylePack::find(&id)
                    .ok_or_else(|| RpcError::new(-32_602, format!("unknown style id '{id}'")))?;
                json!({ "style": pack })
            }
            "get_site" => {
                let available = SiteTerrain::list_sites();
                let site = available
                    .iter()
                    .find(|info| info.id == self.session.site_id())
                    .cloned();
                json!({ "site": site, "available_sites": available })
            }
            "render_plan" => {
                let view = params
                    .get("view")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_PLAN_VIEW);
                let plan_view = match view {
                    "top_plan" => PlanView::TopPlan,
                    "elevation_north" => PlanView::ElevationNorth,
                    other => {
                        return Err(RpcError::new(
                            -32_602,
                            format!("unknown plan view '{other}'"),
                        ))
                    }
                };
                let svg =
                    render_plan_svg(self.session.document(), self.session.terrain(), plan_view);
                json!({
                    "view": view,
                    "document_hash": format!(
                        "{:016x}",
                        self.session.document().document_hash()
                    ),
                    "mime_type": "image/svg+xml",
                    "svg": svg,
                })
            }
            "begin_proposal" => {
                let title = params
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Agent proposal");
                let proposal = self
                    .session
                    .begin_proposal(author, title)
                    .map_err(proposal_error)?;
                json!({ "proposal": proposal_value(&proposal) })
            }
            "stage_proposal_ops" => {
                let proposal_id = required_string(params, "proposal_id")?;
                let ops = operations_from_params(params)?;
                let proposal = self
                    .session
                    .stage_proposal_ops(&proposal_id, &author, ops)
                    .map_err(proposal_error)?;
                json!({ "proposal": proposal_value(&proposal) })
            }
            "submit_proposal" => {
                let proposal_id = required_string(params, "proposal_id")?;
                let summary = params.get("summary").and_then(Value::as_str).unwrap_or("");
                let proposal = self
                    .session
                    .submit_proposal(&proposal_id, &author, summary)
                    .map_err(proposal_error)?;
                json!({ "proposal": proposal_value(&proposal) })
            }
            "list_proposals" => {
                let proposals: Vec<Value> = self.session.proposals().map(proposal_value).collect();
                json!({ "count": proposals.len(), "proposals": proposals })
            }
            "apply_operations" => {
                // B4: direct edits require an active direct-edit participant.
                self.session
                    .ensure_agent_active(&author, AgentPermission::DirectEdit)
                    .map_err(permission_error)?;
                let ops = operations_from_params(params)?;
                let applied_count = ops.len();
                let outcome = self
                    .session
                    .apply_batch(author, ops)
                    .map_err(command_error)?;
                let scene = self.session.build_scene();
                json!({
                    "applied_count": applied_count,
                    "created_ids": outcome.created_ids,
                    "document_hash": format!(
                        "{:016x}",
                        self.session.document().document_hash()
                    ),
                    "layout_hash": format!("{:016x}", scene.layout_hash),
                    "block_count": scene.block_count,
                    "chunk_count": scene.chunks.len(),
                    "summary": outcome.summary,
                })
            }
            "undo" => {
                // B4: undo is a direct edit and needs the same active gate.
                self.session
                    .ensure_agent_active(&author, AgentPermission::DirectEdit)
                    .map_err(permission_error)?;
                let undo = self.session.undo_last(&author).map_err(undo_error)?;
                let scene = self.session.build_scene();
                json!({
                    "seq": undo.undo_seq,
                    "compensated_seq": undo.target_seq,
                    "document_hash": format!(
                        "{:016x}",
                        self.session.document().document_hash()
                    ),
                    "layout_hash": format!("{:016x}", scene.layout_hash),
                    "block_count": scene.block_count,
                    "chunk_count": scene.chunks.len(),
                    "summary": undo.summary,
                })
            }
            "get_history" => {
                let entries: Vec<Value> = self
                    .session
                    .history()
                    .iter()
                    .map(|command| {
                        json!({
                            "seq": command.seq,
                            "command_id": command.command_id,
                            "author": command.author.to_string(),
                            "summary": command.summary,
                            "touched_elements": command.touched_elements,
                            "undone": self.session.is_undone(command.seq),
                        })
                    })
                    .collect();
                json!({ "count": entries.len(), "entries": entries })
            }
            "render_view" => {
                let view_req: keep_proto::RenderViewRequest = if params.is_null() {
                    keep_proto::RenderViewRequest {
                        yaw_deg: None,
                        pitch_deg: None,
                        distance_m: None,
                    }
                } else {
                    serde_json::from_value(params.clone()).map_err(|err| {
                        RpcError::new(-32_602, format!("invalid render_view params: {err}"))
                    })?
                };
                let request_id = self.next_render_view_id;
                self.next_render_view_id += 1;
                return Ok(ControlOutcome::RenderView(request_id, view_req));
            }
            other => return Err(RpcError::new(-32_601, format!("unknown method '{other}'"))),
        };
        Ok(ControlOutcome::Reply(value))
    }
}

/// Rebuild the current session document into a scene dictionary.
impl KeepBridge {
    fn active_style(&self) -> StylePack {
        StylePack::find(&self.style_id).unwrap_or_default()
    }

    fn session_scene(&self) -> VarDictionary {
        let scene = self.session.build_scene();
        let style = self.active_style();
        let mut result = scene_to_dictionary(
            scene,
            self.session.document(),
            self.session.terrain(),
            self.session.document().document_hash(),
            self.session.site_id(),
            &style,
        );
        result.set("history_len", self.session.history().len() as i64);
        result
    }
}

fn error_dictionary(message: &impl std::fmt::Display) -> VarDictionary {
    let mut result = VarDictionary::new();
    result.set("ok", false);
    result.set("error", &GString::from(&message.to_string()));
    result
}

// ---------------------------------------------------------------------------
// Control protocol helpers
// ---------------------------------------------------------------------------

fn required_string(params: &Value, key: &str) -> Result<String, RpcError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| RpcError::new(-32_602, format!("missing string parameter '{key}'")))
}

fn operations_from_params(params: &Value) -> Result<Vec<CommandOp>, RpcError> {
    let operations = params
        .get("operations")
        .ok_or_else(|| RpcError::new(-32_602, "missing operations array"))?;
    serde_json::from_value::<Vec<CommandOp>>(operations.clone())
        .map_err(|error| RpcError::new(-32_602, format!("invalid operations: {error}")))
}

fn command_error(error: keep_core::command::CommandError) -> RpcError {
    RpcError::new(-32_000, error.to_string())
}

fn proposal_error(error: keep_core::proposal::ProposalError) -> RpcError {
    RpcError::new(-32_000, error.to_string())
}

fn permission_error(error: keep_core::proposal::ProposalError) -> RpcError {
    RpcError::new(-32_003, format!("permission denied: {error}"))
}

fn response_error(error: serde_json::Error) -> RpcError {
    RpcError::new(
        -32_000,
        format!("response could not be serialised: {error}"),
    )
}

fn undo_error(error: keep_core::session::UndoError) -> RpcError {
    RpcError::new(-32_000, error.to_string())
}

fn proposal_value(proposal: &Proposal) -> Value {
    serde_json::to_value(proposal).unwrap_or(Value::Null)
}

/// Cheap fingerprint of the proposal set, used to tell `main.gd` when the
/// panel and ghost overlays need rebuilding.
fn proposal_signature(session: &Session) -> Vec<(String, &'static str, usize, String)> {
    session
        .proposals()
        .map(|proposal| {
            (
                proposal.id.clone(),
                status_key(proposal.status),
                proposal.ops.len(),
                proposal.summary.clone(),
            )
        })
        .collect()
}

fn permission_key(permission: AgentPermission) -> &'static str {
    match permission {
        AgentPermission::ReadOnly => "read_only",
        AgentPermission::ProposalOnly => "proposal_only",
        AgentPermission::DirectEdit => "direct_edit",
    }
}

fn permission_from_key(key: &str) -> AgentPermission {
    match key.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "read_only" | "readonly" => AgentPermission::ReadOnly,
        "direct_edit" | "directedit" => AgentPermission::DirectEdit,
        _ => AgentPermission::ProposalOnly,
    }
}

fn state_key(state: AgentState) -> &'static str {
    match state {
        AgentState::Idle => "idle",
        AgentState::Thinking => "thinking",
        AgentState::BuildingProposal => "building_proposal",
        AgentState::WaitingForApproval => "waiting",
        AgentState::Paused => "paused",
    }
}

fn status_key(status: ProposalStatus) -> &'static str {
    match status {
        ProposalStatus::Draft => "draft",
        ProposalStatus::Submitted => "submitted",
        ProposalStatus::Accepted => "accepted",
        ProposalStatus::Rejected => "rejected",
    }
}

fn assessment_json(document: &CastleDocument, terrain: &SiteTerrain, style: &StylePack) -> Value {
    let report = validate_structure(document, terrain, style);
    let historian = assess_historian(document, terrain, style);
    json!({
        "score": report.score,
        "passed": report.passed,
        "style_id": style.id.clone(),
        "findings": report.findings,
        "historian": historian,
    })
}

fn quantise_m(value: f32) -> i64 {
    (f64::from(value) * 1000.0).round() as i64
}

fn quantise_xz(point: Vector2) -> [i64; 2] {
    [quantise_m(point.x), quantise_m(point.y)]
}

fn mm_to_m(value: i64) -> f32 {
    (value as f32) / 1000.0
}

fn snap_label(kind: Option<SnapKind>) -> GString {
    let text = match kind {
        Some(SnapKind::TowerCentre) => "Tower centre",
        Some(SnapKind::TowerPerimeter) => "Tower edge",
        Some(SnapKind::WallEndpoint) => "Wall end",
        None => "",
    };
    GString::from(text)
}

/// Stable machine-readable snap key so the design tool can tell a *new* target
/// apart from the same target under a moving cursor.
fn snap_kind_key(kind: Option<SnapKind>) -> GString {
    let text = match kind {
        Some(SnapKind::TowerCentre) => "tower_centre",
        Some(SnapKind::TowerPerimeter) => "tower_perimeter",
        Some(SnapKind::WallEndpoint) => "wall_endpoint",
        None => "",
    };
    GString::from(text)
}

fn scene_to_dictionary(
    scene: SceneData,
    document: &CastleDocument,
    terrain: &SiteTerrain,
    document_hash: u64,
    site_id: &str,
    style: &StylePack,
) -> VarDictionary {
    let mut result = VarDictionary::new();

    let terrain_mesh = terrain_mesh(&scene.terrain_mesh);
    result.set("terrain", &terrain_mesh);

    let mut chunks: Array<VarDictionary> = Array::new();
    for chunk in &scene.chunks {
        let mut entry = VarDictionary::new();
        entry.set(
            "key",
            Vector3i::new(chunk.key[0], chunk.key[1], chunk.key[2]),
        );
        entry.set(
            "center",
            Vector3::new(chunk.centre[0], chunk.centre[1], chunk.centre[2]),
        );
        entry.set("lod0", &chunk_lod_mesh(&chunk.lod0));
        entry.set("lod1", &chunk_lod_mesh(&chunk.lod1));
        entry.set("lod2", &chunk_lod_mesh(&chunk.lod2));
        entry.set("block_count", chunk.block_count as i64);
        chunks.push(&entry);
    }
    let historian = assess_historian(document, terrain, style);
    result.set("chunks", &chunks);
    result.set("elements", &elements_array(document, terrain));
    result.set("site_id", &GString::from(site_id));
    result.set("style_id", &GString::from(&style.id));
    result.set("material_palette", &GString::from(&style.material_palette));
    result.set("historian", &historian_dictionary(&historian));
    result.set(
        "assessment",
        &assessment_dictionary(document, terrain, style),
    );
    result.set("block_count", scene.block_count as i64);
    result.set("chunk_count", scene.chunks.len() as i64);
    result.set("triangle_count", scene.triangle_count as i64);
    let layout_hash = GString::from(&format!("{:016x}", scene.layout_hash));
    result.set("layout_hash", &layout_hash);
    let document_hash = GString::from(&format!("{document_hash:016x}"));
    result.set("document_hash", &document_hash);
    result
}

/// Per-element plan footprints in metres for the construction FX layer. Each
/// entry carries the terrain-grounded base height (and wall end heights) so the
/// scaffold follows slopes instead of sitting on one global castle AABB.
fn elements_array(document: &CastleDocument, terrain: &SiteTerrain) -> Array<VarDictionary> {
    let mut elements: Array<VarDictionary> = Array::new();
    for element in document.elements() {
        match element {
            Element::BatteredWall(w) => {
                let start_x = w.start_mm[0] as f32 / 1000.0;
                let start_z = w.start_mm[1] as f32 / 1000.0;
                let end_x = w.end_mm[0] as f32 / 1000.0;
                let end_z = w.end_mm[1] as f32 / 1000.0;
                let mid_x = (start_x + end_x) * 0.5;
                let mid_z = (start_z + end_z) * 0.5;
                let mut entry = VarDictionary::new();
                let kind = GString::from("wall");
                entry.set("kind", &kind);
                entry.set("start", Vector2::new(start_x, start_z));
                entry.set("end", Vector2::new(end_x, end_z));
                entry.set("thickness", w.thickness_mm as f32 / 1000.0);
                entry.set("base_y", terrain.height_m(mid_x, mid_z));
                entry.set("start_y", terrain.height_m(start_x, start_z));
                entry.set("end_y", terrain.height_m(end_x, end_z));
                entry.set("height", w.height_mm as f32 / 1000.0);
                elements.push(&entry);
            }
            Element::RoundTower(t) => {
                let cx = t.centre_mm[0] as f32 / 1000.0;
                let cz = t.centre_mm[1] as f32 / 1000.0;
                let mut entry = VarDictionary::new();
                let kind = GString::from("tower");
                entry.set("kind", &kind);
                entry.set("center", Vector2::new(cx, cz));
                entry.set("radius", t.radius_mm as f32 / 1000.0);
                entry.set("base_y", terrain.height_m(cx, cz));
                entry.set("height", t.height_mm as f32 / 1000.0);
                elements.push(&entry);
            }
            Element::Gatehouse(gatehouse) => {
                // Export the two flanking drums as ordinary tower footprints so
                // scaffolding wraps the gatehouse exactly like a standalone tower.
                let radius = gatehouse.tower_radius_mm as f32 / 1000.0;
                let height = gatehouse.height_mm as f32 / 1000.0;
                for centre in gatehouse.flank_tower_centres() {
                    let cx = centre[0] as f32 / 1000.0;
                    let cz = centre[1] as f32 / 1000.0;
                    let mut entry = VarDictionary::new();
                    let kind = GString::from("tower");
                    entry.set("kind", &kind);
                    entry.set("center", Vector2::new(cx, cz));
                    entry.set("radius", radius);
                    entry.set("base_y", terrain.height_m(cx, cz));
                    entry.set("height", height);
                    elements.push(&entry);
                }
            }
            Element::Keep(keep) => {
                // Export the four keep walls as ordinary curtain segments so the
                // construction scaffolding wraps the great tower cleanly.
                let half_w = keep.width_mm as f64 * 0.5;
                let half_d = keep.depth_mm as f64 * 0.5;
                let corners =
                    oriented_rect_corners(keep.centre_mm, keep.orientation_deg, half_w, half_d);
                push_rect_walls(
                    &mut elements,
                    terrain,
                    &corners,
                    keep.wall_thickness_mm as f32 / 1000.0,
                    keep.height_mm as f32 / 1000.0,
                );
            }
            Element::Range(range) => {
                // A roofed range is a rectangle of four walls; export them as
                // curtain segments at eaves height.
                let half_l = range.length_mm as f64 * 0.5;
                let half_w = range.width_mm as f64 * 0.5;
                let corners =
                    oriented_rect_corners(range.centre_mm, range.orientation_deg, half_l, half_w);
                push_rect_walls(
                    &mut elements,
                    terrain,
                    &corners,
                    0.6,
                    range.wall_height_mm as f32 / 1000.0,
                );
            }
            Element::Ditch(_) => {}
        }
    }
    elements
}

/// Four world-space corners of an oriented rectangle (metres).
fn oriented_rect_corners(
    centre_mm: [i64; 2],
    orientation_deg: i32,
    half_u: f64,
    half_v: f64,
) -> [(f32, f32); 4] {
    let cx = centre_mm[0] as f64 / 1000.0;
    let cz = centre_mm[1] as f64 / 1000.0;
    let angle = (orientation_deg as f64).to_radians();
    let (sin_a, cos_a) = (angle.sin(), angle.cos());
    let mut corners = [(0.0f32, 0.0f32); 4];
    for (index, (u, v)) in [
        (-half_u, -half_v),
        (half_u, -half_v),
        (half_u, half_v),
        (-half_u, half_v),
    ]
    .into_iter()
    .enumerate()
    {
        let x = cx + u * cos_a - v * sin_a;
        let z = cz + u * sin_a + v * cos_a;
        corners[index] = (x as f32, z as f32);
    }
    corners
}

/// Emit one wall footprint per rectangle edge so scaffolding and dust follow
/// the element outline.
fn push_rect_walls(
    elements: &mut Array<VarDictionary>,
    terrain: &SiteTerrain,
    corners: &[(f32, f32); 4],
    thickness: f32,
    height: f32,
) {
    for index in 0..4 {
        let (start_x, start_z) = corners[index];
        let (end_x, end_z) = corners[(index + 1) % 4];
        let mid_x = (start_x + end_x) * 0.5;
        let mid_z = (start_z + end_z) * 0.5;
        let mut entry = VarDictionary::new();
        let kind = GString::from("wall");
        entry.set("kind", &kind);
        entry.set("start", Vector2::new(start_x, start_z));
        entry.set("end", Vector2::new(end_x, end_z));
        entry.set("thickness", thickness);
        entry.set("base_y", terrain.height_m(mid_x, mid_z));
        entry.set("start_y", terrain.height_m(start_x, start_z));
        entry.set("end_y", terrain.height_m(end_x, end_z));
        entry.set("height", height);
        elements.push(&entry);
    }
}

/// Structural assessment in the shape the HUD expects, including the
/// historian's deterministic reading of the same document.
fn assessment_dictionary(
    document: &CastleDocument,
    terrain: &SiteTerrain,
    style: &StylePack,
) -> VarDictionary {
    let report = validate_structure(document, terrain, style);
    let historian = assess_historian(document, terrain, style);
    let mut findings: Array<VarDictionary> = Array::new();
    for finding in &report.findings {
        let mut entry = VarDictionary::new();
        entry.set("severity", &severity_key(finding.severity));
        entry.set("code", &GString::from(&finding.code));
        entry.set("element_id", finding.element_id as i64);
        entry.set("message", &GString::from(&finding.message));
        entry.set("source_note", &GString::from(&finding.source_note));
        entry.set("has_fix", finding.suggested_fix.is_some());
        let fix_json = finding
            .suggested_fix
            .as_ref()
            .map(command_op_to_json)
            .unwrap_or_default();
        entry.set("fix_json", &GString::from(&fix_json));
        findings.push(&entry);
    }
    let mut result = VarDictionary::new();
    result.set("score", i64::from(report.score));
    result.set("passed", report.passed);
    result.set("style_id", &GString::from(&style.id));
    result.set("findings", &findings);
    result.set("historian", &historian_dictionary(&historian));
    result
}

fn historian_dictionary(report: &HistorianReport) -> VarDictionary {
    let mut result = VarDictionary::new();
    result.set("style_id", &GString::from(&report.style_id));
    result.set("overall_score", i64::from(report.overall_score));
    result.set("authenticity_score", i64::from(report.authenticity_score));
    result.set("defensive_score", i64::from(report.defensive_score));
    result.set("domestic_score", i64::from(report.domestic_score));
    let mut findings: Array<VarDictionary> = Array::new();
    for finding in &report.findings {
        let mut entry = VarDictionary::new();
        entry.set(
            "category",
            &GString::from(historian_category_key(finding.category)),
        );
        entry.set("code", &GString::from(&finding.code));
        entry.set("message", &GString::from(&finding.message));
        entry.set("source_note", &GString::from(&finding.source_note));
        entry.set("score_impact", i64::from(finding.score_impact));
        findings.push(&entry);
    }
    result.set("findings", &findings);
    result
}

fn historian_category_key(category: HistorianCategory) -> &'static str {
    match category {
        HistorianCategory::Authenticity => "authenticity",
        HistorianCategory::Defensive => "defensive",
        HistorianCategory::Domestic => "domestic",
    }
}

fn range_kind_from_key(key: &str) -> RangeKind {
    match key.trim().to_ascii_lowercase().as_str() {
        "chapel" => RangeKind::Chapel,
        "kitchen" => RangeKind::Kitchen,
        _ => RangeKind::Hall,
    }
}

fn range_kind_name(kind: RangeKind) -> &'static str {
    match kind {
        RangeKind::Hall => "Hall",
        RangeKind::Chapel => "Chapel",
        RangeKind::Kitchen => "Kitchen",
    }
}

fn severity_key(severity: Severity) -> GString {
    GString::from(match severity {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
    })
}

fn tower_plan_from_key(key: &str) -> TowerPlan {
    match key.trim().to_ascii_lowercase().as_str() {
        "square" => TowerPlan::Square,
        "d_shaped" | "d-shaped" | "dshaped" => TowerPlan::DShaped,
        _ => TowerPlan::Round,
    }
}

/// Serialise a [`CommandOp`] to compact JSON for Godot callers.
fn command_op_to_json(op: &CommandOp) -> String {
    serde_json::to_string(op).unwrap_or_default()
}

/// Parse a serialised [`CommandOp`] (the JSON stored in a finding's
/// `fix_json`).
fn command_op_from_json(text: &str) -> Result<CommandOp, String> {
    serde_json::from_str::<CommandOp>(text).map_err(|error| format!("invalid command op: {error}"))
}

fn terrain_mesh(data: &TerrainMeshData) -> Gd<ArrayMesh> {
    let mut mesh = ArrayMesh::new_gd();
    if data.indices.is_empty() {
        return mesh;
    }

    let positions = packed_vec3(&data.positions);
    let normals = packed_vec3(&data.normals);
    let uvs = packed_vec2(&data.uvs);
    let colors = packed_colors(&data.colors);
    let indices = PackedInt32Array::from(data.indices.as_slice());

    let mut arrays = new_vertex_arrays();
    set_array(&mut arrays, ArrayType::VERTEX, &positions);
    set_array(&mut arrays, ArrayType::NORMAL, &normals);
    set_array(&mut arrays, ArrayType::TEX_UV, &uvs);
    set_array(&mut arrays, ArrayType::COLOR, &colors);
    set_array(&mut arrays, ArrayType::INDEX, &indices);

    mesh.add_surface_from_arrays(PrimitiveType::TRIANGLES, &arrays);
    mesh
}

fn chunk_lod_mesh(data: &ChunkMeshData) -> Gd<ArrayMesh> {
    let mut mesh = ArrayMesh::new_gd();
    if data.indices.is_empty() {
        return mesh;
    }

    let positions = packed_vec3(&data.positions);
    let normals = packed_vec3(&data.normals);
    let uvs = packed_vec2(&data.uvs);
    let colors = packed_colors(&data.colors);
    let custom0 = packed_custom0(&data.custom0);
    let indices = PackedInt32Array::from(data.indices.as_slice());

    let mut arrays = new_vertex_arrays();
    set_array(&mut arrays, ArrayType::VERTEX, &positions);
    set_array(&mut arrays, ArrayType::NORMAL, &normals);
    set_array(&mut arrays, ArrayType::TEX_UV, &uvs);
    set_array(&mut arrays, ArrayType::COLOR, &colors);
    set_array(&mut arrays, ArrayType::CUSTOM0, &custom0);
    set_array(&mut arrays, ArrayType::INDEX, &indices);

    // CUSTOM0 carries four floats per vertex (RGBA_FLOAT): build index, course,
    // block width and block height. The shader reads it for the build front.
    let custom_bits =
        (ArrayCustomFormat::RGBA_FLOAT.ord() as u64) << ArrayFormat::CUSTOM0_SHIFT.ord();
    let flags = ArrayFormat::CUSTOM0 | ArrayFormat::from_ord(custom_bits);

    mesh.add_surface_from_arrays_ex(PrimitiveType::TRIANGLES, &arrays)
        .flags(flags)
        .done();
    mesh
}

fn new_vertex_arrays() -> VarArray {
    let mut arrays = VarArray::new();
    arrays.resize_default(ArrayType::MAX.ord() as usize);
    arrays
}

fn set_array(arrays: &mut VarArray, slot: ArrayType, value: &impl godot::meta::ToGodot) {
    arrays.set(slot.ord() as usize, &value.to_variant());
}

fn packed_vec3(values: &[[f32; 3]]) -> PackedVector3Array {
    values
        .iter()
        .map(|v| Vector3::new(v[0], v[1], v[2]))
        .collect()
}

fn packed_vec2(values: &[[f32; 2]]) -> PackedVector2Array {
    values.iter().map(|v| Vector2::new(v[0], v[1])).collect()
}

fn packed_colors(values: &[[f32; 4]]) -> PackedColorArray {
    values
        .iter()
        .map(|v| Color::from_rgba(v[0], v[1], v[2], v[3]))
        .collect()
}

fn packed_custom0(values: &[[f32; 4]]) -> PackedFloat32Array {
    values
        .iter()
        .flat_map(|value| value.iter().copied())
        .collect()
}

/// Register the extension.
struct KeepGd;

#[gdextension]
unsafe impl ExtensionLibrary for KeepGd {}
