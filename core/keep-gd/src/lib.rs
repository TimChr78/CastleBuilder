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
use keep_core::commission::{list_commissions, CommissionState, Season};
use keep_core::coplay::{CoplaySnapshot, HashCheckResult, PlayerRole, GENERATOR_VERSION};
use keep_core::document::{CastleDocument, Element, RangeKind, TerrainBrushKind, TowerPlan};
use keep_core::historian::{assess_historian, HistorianCategory, HistorianReport};
use keep_core::interior::{build_interior_props, SectionPlane};
use keep_core::mesh::{ChunkMeshData, SceneData};
use keep_core::picking::SnapKind;
use keep_core::plan_render::{render_plan_svg_with_section, PlanView};
use keep_core::proposal::{
    ActivityEntry, AgentParticipant, AgentPermission, AgentRequest, AgentState, Proposal,
    ProposalStatus, RequestStatus, MAX_ACTIVITY_ENTRIES,
};
use keep_core::session::Session;
use keep_core::style::StylePack;
use keep_core::terrain::{SiteTerrain, TerrainMeshData};
use keep_core::validate::{validate_structure, Severity};
use keep_proto::{
    build_castle_response, build_element_list_response, ActivityFeedArgs, RenderViewResponse,
    DEFAULT_PLAN_VIEW,
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
    /// Visual construction front in `0.0..=1.0`, used for mid-build salvage.
    build_front_norm: f32,
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
            build_front_norm: 1.0,
        };
        bridge.ensure_control_agents();
        bridge.sync_style_id();
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
        self.sync_style_id();
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

    /// Describe every historical codex entry (M3.1).
    #[func]
    fn list_codex_entries(&self) -> Array<VarDictionary> {
        let mut entries: Array<VarDictionary> = Array::new();
        for entry in keep_core::codex::list_codex_entries() {
            let mut item = VarDictionary::new();
            item.set("id", &GString::from(&entry.id));
            item.set("title", &GString::from(&entry.title));
            item.set("category", &GString::from(&entry.category));
            item.set("period", &GString::from(&entry.period));
            item.set("summary", &GString::from(&entry.summary));
            item.set("body", &GString::from(&entry.body));
            let mut citations: Array<GString> = Array::new();
            for citation in &entry.citations {
                citations.push(&GString::from(citation));
            }
            item.set("citations", &citations);
            let mut related: Array<GString> = Array::new();
            for style_id in &entry.related_style_ids {
                related.push(&GString::from(style_id));
            }
            item.set("related_style_ids", &related);
            entries.push(&item);
        }
        entries
    }

    // -----------------------------------------------------------------------
    // M3.2: commissions, seasons and mid-build salvage
    // -----------------------------------------------------------------------

    /// Describe every curated commission brief.
    #[func]
    fn list_commissions(&self) -> Array<VarDictionary> {
        let mut commissions: Array<VarDictionary> = Array::new();
        for brief in list_commissions() {
            commissions.push(&commission_brief_dictionary(&brief));
        }
        commissions
    }

    /// Load a curated commission, switching to its site and style and clearing
    /// the castle. The returned scene also carries the live commission state.
    #[func]
    fn start_commission(&mut self, commission_id: GString) -> VarDictionary {
        let id = commission_id.to_string();
        match self.session.start_commission(&id) {
            Ok(state) => {
                self.style_id = self.session.style_id().to_string();
                self.sync_style_id();
                // A commission resets the site/style and castle but must keep the
                // live MCP and demo agents available for proposals (M3.2).
                self.ensure_control_agents();
                let mut result = self.session_scene();
                result.set("ok", true);
                result.set("commission", &commission_dictionary(&state));
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Leave commission mode and return to the free-build sandbox.
    #[func]
    fn end_commission(&mut self) -> VarDictionary {
        self.session.end_commission();
        let mut result = VarDictionary::new();
        result.set("ok", true);
        result
    }

    /// Advance one season, applying frost, the deterministic event and upkeep.
    #[func]
    fn advance_season(&mut self) -> VarDictionary {
        match self.session.advance_season() {
            Ok(state) => {
                let mut result = VarDictionary::new();
                result.set("ok", true);
                result.set("commission", &commission_dictionary(&state));
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Current commission state: `{ active, commission? }`.
    #[func]
    fn get_commission_state(&self) -> VarDictionary {
        let mut result = VarDictionary::new();
        match self.session.commission() {
            Some(state) => {
                result.set("active", true);
                result.set("commission", &commission_dictionary(state));
            }
            None => {
                result.set("active", false);
            }
        }
        result
    }

    /// Tell the session how much of the castle is visually built so an edit can
    /// credit the stone it dismantles (M3.2 mid-build salvage).
    #[func]
    fn set_build_front_norm(&mut self, front: f32) {
        self.build_front_norm = front.clamp(0.0, 1.0);
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
            &self.session.effective_terrain(),
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
        match self.apply_edit(vec![op]) {
            Ok(mut result) => {
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
        match self.apply_edit(vec![op]) {
            Ok(mut result) => {
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
        match self.apply_edit(ops) {
            Ok(mut result) => {
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
        match self.apply_edit(vec![op]) {
            Ok(mut result) => {
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
        match self.apply_edit(vec![op]) {
            Ok(mut result) => {
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
        match self.apply_edit(vec![op]) {
            Ok(mut result) => {
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
        match self.apply_edit(vec![op]) {
            Ok(mut result) => {
                result.set("ok", true);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Sculpt the site terrain with an authoritative brush (M3.1).
    ///
    /// `centre_xz`, `radius_m` and `delta_m` are metres; everything is
    /// quantised to integer millimetres so the edit hashes and undoes exactly
    /// like any other command. `flatten` treats `delta_m` as the target
    /// elevation and `smooth` as a strength in metres (0.65 ≈ 65%).
    #[func]
    fn sculpt_terrain(
        &mut self,
        kind: GString,
        centre_xz: Vector2,
        radius_m: f32,
        delta_m: f32,
    ) -> VarDictionary {
        let brush = terrain_brush_from_key(&kind.to_string());
        let op = CommandOp::SculptTerrain {
            ref_id: None,
            name: format!(
                "{} {}",
                terrain_brush_name(brush),
                self.session.next_element_id()
            ),
            kind: brush,
            centre_mm: quantise_xz(centre_xz),
            radius_mm: quantise_m(radius_m),
            delta_mm: quantise_m(delta_m),
        };
        match self.apply_edit(vec![op]) {
            Ok(mut result) => {
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
        match self.apply_edit(vec![op]) {
            Ok(mut result) => {
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
                // Undo must not award mid-build salvage (B1): the restored
                // masonry is already charged in the core session, so rebuild
                // the scene without running the salvage diff.
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
    // M5 Release 2: co-play players, presence, claims and wire messages
    // -----------------------------------------------------------------------

    /// Describe the live co-play session: player limit, participants, soft
    /// claims, the document hash and the protocol version.
    #[func]
    fn coplay_state(&self) -> VarDictionary {
        let mut result = VarDictionary::new();
        result.set("max_players", self.session.max_players() as i64);

        let mut players: Array<VarDictionary> = Array::new();
        for player in self.session.players() {
            players.push(&player_dictionary(player));
        }
        result.set("players", &players);

        let mut claims: Array<VarDictionary> = Array::new();
        for claim in self.session.claims() {
            claims.push(&claim_dictionary(claim));
        }
        result.set("claims", &claims);

        result.set(
            "document_hash",
            &GString::from(&format!("{:016x}", self.session.document().document_hash())),
        );
        result.set("generator_version", i64::from(GENERATOR_VERSION));
        result
    }

    /// Set the host player limit (clamped by the core to `1..=4`) and return
    /// the resulting cap.
    #[func]
    fn coplay_set_max_players(&mut self, max_players: i64) -> i64 {
        self.session.set_max_players(max_players.max(1) as usize);
        self.session.max_players() as i64
    }

    /// Register or refresh a player. Roles are `"host"`, `"builder"` or
    /// `"viewer"`.
    #[func]
    fn coplay_register_player(
        &mut self,
        id: GString,
        name: GString,
        color_hex: GString,
        role: GString,
    ) -> VarDictionary {
        let role = match parse_player_role(&role.to_string()) {
            Ok(role) => role,
            Err(message) => return error_dictionary(&message),
        };
        match self.session.register_player(
            &id.to_string(),
            &name.to_string(),
            &color_hex.to_string(),
            role,
        ) {
            Ok(player) => {
                let mut result = VarDictionary::new();
                result.set("ok", true);
                result.set("player", &player_dictionary(player));
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Disconnect a player and drop every soft claim they own.
    #[func]
    fn coplay_remove_player(&mut self, id: GString) -> bool {
        self.session.remove_player(&id.to_string())
    }

    /// Change a registered player's role.
    #[func]
    fn coplay_set_player_role(&mut self, id: GString, role: GString) -> VarDictionary {
        let role = match parse_player_role(&role.to_string()) {
            Ok(role) => role,
            Err(message) => return error_dictionary(&message),
        };
        match self.session.set_player_role(&id.to_string(), role) {
            Ok(()) => {
                let mut result = VarDictionary::new();
                result.set("ok", true);
                match self.session.player(&id.to_string()) {
                    Some(player) => result.set("player", &player_dictionary(player)),
                    None => result.set("player", &VarDictionary::new()),
                }
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Update a player's camera marker and status. Metres are converted to the
    /// quantized integer millimetres the core stores.
    #[func]
    fn coplay_update_presence(
        &mut self,
        id: GString,
        cam_pos: Vector3,
        cam_target: Vector3,
        status: GString,
    ) -> bool {
        self.session.update_player_presence(
            &id.to_string(),
            metres_to_mm(cam_pos),
            metres_to_mm(cam_target),
            &status.to_string(),
        )
    }

    /// Place a non-blocking soft claim on an element.
    #[func]
    fn coplay_claim_element(
        &mut self,
        element_id: i64,
        player_id: GString,
        note: GString,
    ) -> VarDictionary {
        let author = match AuthorId::player(player_id.to_string()) {
            Ok(author) => author,
            Err(error) => return error_dictionary(&error),
        };
        match self
            .session
            .claim_element(element_id.max(0) as u64, author, &note.to_string())
        {
            Ok(claim) => {
                let mut result = VarDictionary::new();
                result.set("ok", true);
                result.set("claim", &claim_dictionary(&claim));
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Release a soft claim. The owner or the host may release.
    #[func]
    fn coplay_release_claim(&mut self, element_id: i64, player_id: GString) -> bool {
        let Ok(author) = AuthorId::player(player_id.to_string()) else {
            return false;
        };
        self.session
            .release_claim(element_id.max(0) as u64, &author)
    }

    /// Serialise the full co-play snapshot as JSON, or an empty string when it
    /// cannot be encoded.
    #[func]
    fn coplay_snapshot_json(&self) -> GString {
        match serde_json::to_string(&self.session.build_coplay_snapshot()) {
            Ok(json) => GString::from(&json),
            Err(_) => GString::new(),
        }
    }

    /// Replace the live session from a host snapshot, validating version, site,
    /// style pack and document first.
    #[func]
    fn coplay_apply_snapshot_json(&mut self, json: GString) -> VarDictionary {
        let snapshot: CoplaySnapshot = match serde_json::from_str(&json.to_string()) {
            Ok(snapshot) => snapshot,
            Err(error) => return error_dictionary(&format!("invalid snapshot JSON: {error}")),
        };
        match Session::from_coplay_snapshot(snapshot) {
            Ok(session) => {
                self.style_id = session.style_id().to_string();
                self.replace_session(session);
                let mut result = self.session_scene();
                result.set("ok", true);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Apply an ordered remote batch after the shared edit gate. Returns the
    /// rebuilt scene plus the assigned sequence number and document hash.
    #[func]
    fn coplay_apply_remote_batch_json(
        &mut self,
        author_id: GString,
        ops_json: GString,
    ) -> VarDictionary {
        let author = match AuthorId::player(author_id.to_string()) {
            Ok(author) => author,
            Err(error) => return error_dictionary(&error),
        };
        let ops: Vec<CommandOp> = match serde_json::from_str(&ops_json.to_string()) {
            Ok(ops) => ops,
            Err(error) => return error_dictionary(&format!("invalid operations JSON: {error}")),
        };
        if let Err(error) = self.session.ensure_player_can_edit(&author) {
            return error_dictionary(&error);
        }
        match self.session.apply_batch(author, ops) {
            Ok(outcome) => {
                let mut result = self.session_scene();
                result.set("ok", true);
                result.set("seq", outcome.seq as i64);
                result.set(
                    "document_hash",
                    &GString::from(&format!("{:016x}", outcome.document_hash)),
                );
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Apply a remote undo through the shared edit gate.
    #[func]
    fn coplay_apply_remote_undo(&mut self, author_id: GString) -> VarDictionary {
        let author = match AuthorId::player(author_id.to_string()) {
            Ok(author) => author,
            Err(error) => return error_dictionary(&error),
        };
        if let Err(error) = self.session.ensure_player_can_edit(&author) {
            return error_dictionary(&error);
        }
        match self.session.undo_last(&author) {
            Ok(undo) => {
                let mut result = self.session_scene();
                result.set("ok", true);
                result.set("undone", true);
                result.set("target_seq", undo.target_seq as i64);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }

    /// Compare the host's document and layout hashes against the live session
    /// and classify the outcome.
    #[func]
    fn coplay_verify_hashes(
        &self,
        host_doc_hash: GString,
        host_layout_hash: GString,
    ) -> VarDictionary {
        hash_check_dictionary(
            &self
                .session
                .verify_coplay_hashes(&host_doc_hash.to_string(), &host_layout_hash.to_string()),
        )
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
            let before_panel = panel_signature(&self.session);
            let outcome = self.execute_control(&method, &params);
            if self.session.document().document_hash() != before_hash {
                dirty = true;
            }
            if panel_signature(&self.session) != before_panel {
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
            entry.set("historian_before", i64::from(proposal.historian_before));
            entry.set("historian_after", i64::from(proposal.historian_after));
            entry.set("historian_delta", i64::from(proposal.historian_delta));
            entry.set("stale", self.session.is_proposal_stale(&proposal.id));
            entry.set("status", &GString::from(status_key(proposal.status)));
            entry.set(
                "reject_reason",
                &GString::from(proposal.reject_reason.as_deref().unwrap_or("")),
            );
            entry.set("op_count", proposal.ops.len() as i64);
            proposals.push(&entry);
        }
        result.set("proposals", &proposals);

        let mut requests: Array<VarDictionary> = Array::new();
        for request in self.session.requests() {
            requests.push(&request_dictionary(request));
        }
        result.set("requests", &requests);

        // Newest first, so the panel can render the feed directly.
        let mut activity: Array<VarDictionary> = Array::new();
        for entry in self.session.activity_feed().iter().rev().take(25) {
            activity.push(&activity_dictionary(entry));
        }
        result.set("activity", &activity);
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
        let before = self.session.document().clone();
        match self.session.accept_proposal(&proposal_id.to_string()) {
            Ok(_) => {
                let mut result = self.scene_after_edit(before);
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

    /// Pause an agent: every proposal, command and request mutation from it is
    /// refused until it is resumed (architecture section 7.7).
    #[func]
    fn pause_agent(&mut self, agent_id: GString) -> VarDictionary {
        let id = agent_id.to_string();
        match self.session.pause_agent(&id) {
            Ok(()) => agent_state_dictionary(&self.session, &id),
            Err(error) => error_dictionary(&error),
        }
    }

    /// Resume a paused agent. An agent that still has a submitted proposal goes
    /// back to waiting for approval, otherwise it returns to idle.
    #[func]
    fn resume_agent(&mut self, agent_id: GString) -> VarDictionary {
        let id = agent_id.to_string();
        match self.session.resume_agent(&id) {
            Ok(()) => agent_state_dictionary(&self.session, &id),
            Err(error) => error_dictionary(&error),
        }
    }

    /// Disconnect and remove an agent from the session.
    #[func]
    fn disconnect_agent(&mut self, agent_id: GString) -> VarDictionary {
        let id = agent_id.to_string();
        let removed = self.session.disconnect_agent(&id);
        let mut result = VarDictionary::new();
        result.set("ok", removed);
        result.set("disconnected", removed);
        result.set("agent_id", &agent_id);
        if !removed {
            result.set("error", &GString::from("unknown agent"));
        }
        result
    }

    /// Queue a human request for the agent inbox. An empty proposal id and a
    /// non-positive pinned element both mean "none".
    #[func]
    fn submit_request(
        &mut self,
        prompt: GString,
        proposal_id: GString,
        pinned_element_id: i64,
    ) -> VarDictionary {
        let prompt = prompt.to_string();
        let proposal = proposal_id.to_string();
        let proposal_ref = if proposal.trim().is_empty() {
            None
        } else {
            Some(proposal.as_str())
        };
        let pinned = if pinned_element_id <= 0 {
            None
        } else {
            Some(pinned_element_id as u64)
        };
        let author = self.player_author.clone();
        match self
            .session
            .submit_request(author, &prompt, proposal_ref, pinned)
        {
            Ok(request) => request_dictionary_with_ok(&request),
            Err(error) => error_dictionary(&error),
        }
    }

    /// Ask a submitted proposal's author for changes by queuing a linked
    /// request.
    #[func]
    fn ask_for_changes(&mut self, proposal_id: GString, feedback: GString) -> VarDictionary {
        let proposal_id = proposal_id.to_string();
        let feedback = feedback.to_string();
        let author = self.player_author.clone();
        match self
            .session
            .ask_for_changes(&proposal_id, author, &feedback)
        {
            Ok(request) => request_dictionary_with_ok(&request),
            Err(error) => error_dictionary(&error),
        }
    }

    /// Claim an open request as the given agent.
    #[func]
    fn ack_request(&mut self, request_id: GString, agent_id: GString) -> VarDictionary {
        let request_id = request_id.to_string();
        let agent_id = agent_id.to_string();
        match self.session.ack_request(&request_id, &agent_id) {
            Ok(request) => request_dictionary_with_ok(&request),
            Err(error) => error_dictionary(&error),
        }
    }

    /// Resolve an open or acknowledged request, optionally recording a reply.
    #[func]
    fn resolve_request(
        &mut self,
        request_id: GString,
        agent_id: GString,
        reply: GString,
    ) -> VarDictionary {
        let request_id = request_id.to_string();
        let agent_id = agent_id.to_string();
        let reply = reply.to_string();
        let reply = if reply.trim().is_empty() {
            None
        } else {
            Some(reply.as_str())
        };
        match self.session.resolve_request(&request_id, &agent_id, reply) {
            Ok(request) => request_dictionary_with_ok(&request),
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
    /// Keep the session's validation/historian style pack aligned with the
    /// style the bridge last selected. Sessions are replaced on every showcase
    /// or site load, so the id must be re-applied after each swap.
    fn sync_style_id(&mut self) {
        let style_id = self.style_id.clone();
        let _ = self.session.set_style_id(style_id);
    }

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
        self.sync_style_id();
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
            "get_castle" => {
                let value = serde_json::to_value(build_castle_response(&self.session))
                    .map_err(response_error)?;
                self.session.record_activity(
                    format!("agent:{CONTROL_AGENT_ID}"),
                    method,
                    "read the castle state",
                );
                value
            }
            "list_elements" => {
                let value = serde_json::to_value(build_element_list_response(&self.session))
                    .map_err(response_error)?;
                self.session.record_activity(
                    format!("agent:{CONTROL_AGENT_ID}"),
                    method,
                    "read the element list",
                );
                value
            }
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
                let value =
                    assessment_json(self.session.document(), self.session.terrain(), &style);
                self.session.record_activity(
                    format!("agent:{CONTROL_AGENT_ID}"),
                    method,
                    format!("assessed the castle as {}", style.id),
                );
                value
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
                let section = match params.get("section") {
                    None | Some(Value::Null) => SectionPlane::Off,
                    Some(value) => {
                        serde_json::from_value::<SectionPlane>(value.clone()).map_err(|error| {
                            RpcError::new(-32_602, format!("invalid section: {error}"))
                        })?
                    }
                };
                let svg = render_plan_svg_with_section(
                    self.session.document(),
                    self.session.terrain(),
                    plan_view,
                    section,
                );
                self.session.record_activity(
                    format!("agent:{CONTROL_AGENT_ID}"),
                    method,
                    format!("rendered the {view} plan"),
                );
                json!({
                    "view": view,
                    "section": section,
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
            "list_requests" => {
                let requests: Vec<Value> = self.session.requests().map(request_value).collect();
                json!({ "count": requests.len(), "requests": requests })
            }
            "ack_request" => {
                let request_id = required_string(params, "request_id")?;
                let request = self
                    .session
                    .ack_request(&request_id, CONTROL_AGENT_ID)
                    .map_err(request_error)?;
                json!({ "request": request_value(&request) })
            }
            "reply_request" => {
                let request_id = required_string(params, "request_id")?;
                let reply = params.get("reply").and_then(Value::as_str);
                let request = self
                    .session
                    .resolve_request(&request_id, CONTROL_AGENT_ID, reply)
                    .map_err(request_error)?;
                json!({ "request": request_value(&request) })
            }
            "get_activity_feed" => {
                // Share the `keep-proto` contract with the offline sidecar: an
                // omitted limit defaults to 50, 0 returns no entries, negatives
                // are invalid and the feed bound caps the request at 200.
                let args: ActivityFeedArgs = if params.is_null() {
                    ActivityFeedArgs::default()
                } else {
                    serde_json::from_value(params.clone()).map_err(|error| {
                        RpcError::new(
                            -32_602,
                            format!("invalid get_activity_feed params: {error}"),
                        )
                    })?
                };
                let limit = args.limit.unwrap_or(50).min(MAX_ACTIVITY_ENTRIES);
                let feed = self.session.activity_feed();
                let start = feed.len().saturating_sub(limit);
                let entries: Vec<Value> = feed[start..].iter().map(activity_value).collect();
                json!({ "count": entries.len(), "entries": entries })
            }
            "get_commission_status" => json!({
                "active": self.session.commission().is_some(),
                "commission": self.session.commission(),
            }),
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
                self.session.record_activity(
                    format!("agent:{CONTROL_AGENT_ID}"),
                    method,
                    "captured a perspective view",
                );
                return Ok(ControlOutcome::RenderView(request_id, view_req));
            }
            other => return Err(RpcError::new(-32_601, format!("unknown method '{other}'"))),
        };
        Ok(ControlOutcome::Reply(value))
    }
}

/// Rebuild the current session document into a scene dictionary.
impl KeepBridge {
    /// Apply one interactive batch and return the rebuilt scene. While the
    /// construction front is mid-build, the stone dismantled by the edit is
    /// credited back to the active commission and reported to the UI.
    fn apply_edit(
        &mut self,
        ops: Vec<CommandOp>,
    ) -> Result<VarDictionary, keep_core::command::CommandError> {
        let before = self.session.document().clone();
        self.session.apply_batch(self.player_author.clone(), ops)?;
        Ok(self.scene_after_edit(before))
    }

    /// Rebuild the scene after an edit, attaching mid-build salvage feedback
    /// when the construction front is neither bare ground nor finished.
    fn scene_after_edit(&mut self, before: CastleDocument) -> VarDictionary {
        let mut result = self.session_scene();
        // Only an active commission accounts for mid-build salvage (M1). In
        // sandbox mode the extra masonry diff is pure cost with no effect.
        if self.session.commission().is_some()
            && self.build_front_norm > 0.01
            && self.build_front_norm < 0.99
        {
            let diff = keep_core::salvage::diff_mid_build_edit(
                &before,
                self.session.document(),
                self.session.terrain(),
                self.build_front_norm,
            );
            self.session.record_mid_build_salvage(&diff);
            result.set("salvage", &salvage_dictionary(&diff));
        }
        result
    }

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
// Co-play helpers
// ---------------------------------------------------------------------------

fn parse_player_role(role: &str) -> Result<PlayerRole, String> {
    match role.trim().to_ascii_lowercase().as_str() {
        "host" => Ok(PlayerRole::Host),
        "builder" => Ok(PlayerRole::Builder),
        "viewer" => Ok(PlayerRole::Viewer),
        other => Err(format!(
            "unknown player role '{other}' (expected host, builder or viewer)"
        )),
    }
}

fn player_role_name(role: PlayerRole) -> &'static str {
    match role {
        PlayerRole::Host => "host",
        PlayerRole::Builder => "builder",
        PlayerRole::Viewer => "viewer",
    }
}

fn millimetre_triple(value: [i64; 3]) -> Array<i64> {
    let mut array: Array<i64> = Array::new();
    for component in value {
        array.push(component);
    }
    array
}

fn player_dictionary(player: &keep_core::coplay::PlayerParticipant) -> VarDictionary {
    let mut result = VarDictionary::new();
    result.set("id", &GString::from(&player.id));
    result.set("name", &GString::from(&player.name));
    result.set("color_hex", &GString::from(&player.color_hex));
    result.set("role", &GString::from(player_role_name(player.role)));
    result.set("camera_pos_mm", &millimetre_triple(player.camera_pos_mm));
    result.set(
        "camera_target_mm",
        &millimetre_triple(player.camera_target_mm),
    );
    result.set("status", &GString::from(&player.status));
    result
}

fn claim_dictionary(claim: &keep_core::coplay::SoftClaim) -> VarDictionary {
    let mut result = VarDictionary::new();
    result.set("element_id", claim.element_id as i64);
    result.set("owner", &GString::from(&claim.owner.to_string()));
    result.set("note", &GString::from(&claim.note));
    result
}

fn hash_check_dictionary(result: &HashCheckResult) -> VarDictionary {
    let mut dictionary = VarDictionary::new();
    match result {
        HashCheckResult::Match => {
            dictionary.set("kind", &GString::from("match"));
        }
        HashCheckResult::DocumentMismatch {
            host_doc_hash,
            client_doc_hash,
        } => {
            dictionary.set("kind", &GString::from("document_mismatch"));
            dictionary.set("host_doc_hash", &GString::from(host_doc_hash));
            dictionary.set("client_doc_hash", &GString::from(client_doc_hash));
        }
        HashCheckResult::GeneratorDivergence {
            doc_hash,
            host_layout_hash,
            client_layout_hash,
        } => {
            dictionary.set("kind", &GString::from("generator_divergence"));
            dictionary.set("doc_hash", &GString::from(doc_hash));
            dictionary.set("host_layout_hash", &GString::from(host_layout_hash));
            dictionary.set("client_layout_hash", &GString::from(client_layout_hash));
        }
    }
    dictionary
}

fn metres_to_mm(value: Vector3) -> [i64; 3] {
    [
        (value.x * 1000.0).round() as i64,
        (value.y * 1000.0).round() as i64,
        (value.z * 1000.0).round() as i64,
    ]
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

fn request_error(error: keep_core::proposal::RequestError) -> RpcError {
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
    let mut value =
        serde_json::to_value(proposal).unwrap_or_else(|_| Value::Object(Default::default()));
    if let Value::Object(map) = &mut value {
        map.insert(
            "historian_before".to_string(),
            json!(proposal.historian_before),
        );
        map.insert(
            "historian_after".to_string(),
            json!(proposal.historian_after),
        );
        map.insert(
            "historian_delta".to_string(),
            json!(proposal.historian_delta),
        );
    }
    value
}

fn request_value(request: &AgentRequest) -> Value {
    serde_json::to_value(request).unwrap_or(Value::Null)
}

fn activity_value(entry: &ActivityEntry) -> Value {
    serde_json::to_value(entry).unwrap_or(Value::Null)
}

/// Agent request in the shape the in-game panel consumes.
fn request_dictionary(request: &AgentRequest) -> VarDictionary {
    let mut entry = VarDictionary::new();
    entry.set("id", &GString::from(&request.id));
    entry.set("author", &GString::from(&request.author.to_string()));
    entry.set("prompt", &GString::from(&request.prompt));
    entry.set(
        "proposal_id",
        &GString::from(request.proposal_id.as_deref().unwrap_or("")),
    );
    entry.set(
        "pinned_element_id",
        request.pinned_element_id.map(|id| id as i64).unwrap_or(-1),
    );
    entry.set("status", &GString::from(request_status_key(request.status)));
    entry.set(
        "claimed_by",
        &GString::from(request.claimed_by.as_deref().unwrap_or("")),
    );
    entry.set(
        "reply",
        &GString::from(request.reply.as_deref().unwrap_or("")),
    );
    entry.set("created_seq", request.created_seq as i64);
    entry
}

fn request_dictionary_with_ok(request: &AgentRequest) -> VarDictionary {
    let mut result = request_dictionary(request);
    result.set("ok", true);
    result.set("request_id", &GString::from(&request.id));
    result.set("status", &GString::from(request_status_key(request.status)));
    result
}

fn activity_dictionary(entry: &ActivityEntry) -> VarDictionary {
    let mut result = VarDictionary::new();
    result.set("seq", entry.seq as i64);
    result.set("actor", &GString::from(&entry.actor));
    result.set("action", &GString::from(&entry.action));
    result.set("detail", &GString::from(&entry.detail));
    result
}

fn agent_state_dictionary(session: &Session, agent_id: &str) -> VarDictionary {
    let state = session
        .agent(agent_id)
        .map(|agent| state_key(agent.state))
        .unwrap_or("idle");
    let mut result = VarDictionary::new();
    result.set("ok", true);
    result.set("agent_id", &GString::from(agent_id));
    result.set("state", &GString::from(state));
    result
}

fn request_status_key(status: RequestStatus) -> &'static str {
    match status {
        RequestStatus::Open => "open",
        RequestStatus::Acknowledged => "acknowledged",
        RequestStatus::Resolved => "resolved",
    }
}

/// Fingerprint of everything the agent panel renders (proposals, requests,
/// agent presence/permissions and the latest activity entry). `main.gd` uses a
/// change in this signature to rebuild the panel and ghost overlays, so agent
/// reads, acknowledgements and replies are reflected immediately.
#[derive(PartialEq, Eq)]
struct PanelSignature {
    proposals: Vec<(String, &'static str, usize, String)>,
    requests: Vec<(String, &'static str, String, String)>,
    agents: Vec<(String, &'static str, &'static str)>,
    latest_activity_seq: u64,
}

fn panel_signature(session: &Session) -> PanelSignature {
    PanelSignature {
        proposals: session
            .proposals()
            .map(|proposal| {
                (
                    proposal.id.clone(),
                    status_key(proposal.status),
                    proposal.ops.len(),
                    proposal.summary.clone(),
                )
            })
            .collect(),
        requests: session
            .requests()
            .map(|request| {
                (
                    request.id.clone(),
                    request_status_key(request.status),
                    request.claimed_by.clone().unwrap_or_default(),
                    request.reply.clone().unwrap_or_default(),
                )
            })
            .collect(),
        agents: session
            .agents()
            .map(|agent| {
                (
                    agent.id.clone(),
                    permission_key(agent.permission),
                    state_key(agent.state),
                )
            })
            .collect(),
        latest_activity_seq: session.activity_feed().last().map_or(0, |entry| entry.seq),
    }
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
    result.set("interior_props", &interior_props_array(document, terrain));
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

/// Deterministic dollhouse furniture for every keep and roofed range (M3.1).
/// Positions are world-space metres; the UI builder owns all mesh generation.
fn interior_props_array(document: &CastleDocument, terrain: &SiteTerrain) -> Array<VarDictionary> {
    let mut props: Array<VarDictionary> = Array::new();
    for prop in build_interior_props(document, terrain) {
        let mut entry = VarDictionary::new();
        entry.set("kind", &GString::from(prop.kind.kind_name()));
        entry.set("room_type", &GString::from(&prop.room_type));
        entry.set(
            "position",
            Vector3::new(
                mm_to_m(prop.pos_mm[0]),
                mm_to_m(prop.pos_mm[1]),
                mm_to_m(prop.pos_mm[2]),
            ),
        );
        entry.set("yaw_deg", i64::from(prop.yaw_deg));
        entry.set("element_id", prop.element_id as i64);
        props.push(&entry);
    }
    props
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
            Element::Ditch(_) | Element::TerrainSculpt(_) => {}
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

/// One curated commission brief in the shape the picker renders.
fn commission_brief_dictionary(brief: &keep_core::commission::CommissionBrief) -> VarDictionary {
    let mut entry = VarDictionary::new();
    entry.set("id", &GString::from(&brief.id));
    entry.set("title", &GString::from(&brief.title));
    entry.set("patron", &GString::from(&brief.patron));
    entry.set("description", &GString::from(&brief.description));
    entry.set("site_id", &GString::from(&brief.site_id));
    entry.set("style_id", &GString::from(&brief.style_id));
    entry.set("starting_silver", brief.starting_silver);
    entry.set("season_limit", i64::from(brief.season_limit));
    entry.set("starting_workforce", i64::from(brief.starting_workforce));
    entry.set("min_structure_score", i64::from(brief.min_structure_score));
    entry.set("min_historian_score", i64::from(brief.min_historian_score));
    entry.set("min_towers", brief.min_towers as i64);
    entry.set("min_walls", brief.min_walls as i64);
    entry
}

/// Live commission state: the brief plus the seasonal ledger and goal checklist.
fn commission_dictionary(state: &CommissionState) -> VarDictionary {
    let mut result = commission_brief_dictionary(&state.brief);
    result.set("year", i64::from(state.year));
    result.set("season", &GString::from(season_key(state.season)));
    result.set("seasons_elapsed", i64::from(state.seasons_elapsed));
    result.set("treasury_silver", state.treasury_silver);
    result.set("spent_silver", state.spent_silver);
    result.set("salvage_silver", state.salvage_silver);
    result.set("workforce", i64::from(state.workforce));
    result.set("patron_patience", i64::from(state.patron_patience));
    result.set("frost_active", state.frost_active);
    result.set("walls_thatched", state.walls_thatched);

    let mut event = VarDictionary::new();
    if let Some(active) = &state.active_event {
        event.set("id", &GString::from(&active.id));
        event.set("title", &GString::from(&active.title));
        event.set("description", &GString::from(&active.description));
        event.set("silver_delta", active.silver_delta);
        event.set("patience_delta", i64::from(active.patience_delta));
        event.set("workforce_delta", i64::from(active.workforce_delta));
    }
    result.set("active_event", &event);

    let mut goals: Array<VarDictionary> = Array::new();
    for goal in &state.goals {
        let mut entry = VarDictionary::new();
        entry.set("id", &GString::from(&goal.id));
        entry.set("label", &GString::from(&goal.label));
        entry.set("met", goal.met);
        goals.push(&entry);
    }
    result.set("goals", &goals);
    result.set("completed", state.completed);
    result.set("failed", state.failed);
    result.set("status_note", &GString::from(&state.status_note));
    result
}

fn season_key(season: Season) -> &'static str {
    match season {
        Season::Spring => "spring",
        Season::Summer => "summer",
        Season::Autumn => "autumn",
        Season::Winter => "winter",
    }
}

/// Mid-build salvage report consumed by the HUD and the headless tests.
fn salvage_dictionary(diff: &keep_core::salvage::SalvageDiff) -> VarDictionary {
    let mut result = VarDictionary::new();
    result.set("unchanged_built_blocks", diff.unchanged_built_blocks as i64);
    result.set("dismantled_blocks", diff.dismantled_blocks as i64);
    result.set("added_blocks", diff.added_blocks as i64);
    result.set("salvage_credit_silver", diff.salvage_credit_silver);
    result.set("preserved_fraction", f64::from(diff.preserved_fraction));
    result
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

/// Map the UI's brush key to the authoritative terrain brush kind.
fn terrain_brush_from_key(key: &str) -> TerrainBrushKind {
    match key.trim().to_ascii_lowercase().as_str() {
        "lower" | "dig" => TerrainBrushKind::Lower,
        "flatten" | "level" => TerrainBrushKind::Flatten,
        "smooth" | "blend" => TerrainBrushKind::Smooth,
        _ => TerrainBrushKind::Raise,
    }
}

fn terrain_brush_name(kind: TerrainBrushKind) -> &'static str {
    match kind {
        TerrainBrushKind::Raise => "Raise",
        TerrainBrushKind::Lower => "Lower",
        TerrainBrushKind::Flatten => "Flatten",
        TerrainBrushKind::Smooth => "Smooth",
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
