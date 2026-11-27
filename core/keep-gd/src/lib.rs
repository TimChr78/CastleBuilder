//! Thin GDExtension binding: commands in, mesh buffers out.
//!
//! `KeepBridge` is a thin *session* bridge, not an authority: it forwards
//! interactive edits to a [`keep_core::session::Session`], which owns the
//! authoritative [`keep_core::document::CastleDocument`], the append-only
//! command log and the undo semantics. Every rebuild still runs the
//! deterministic `keep-core` pipeline. The returned dictionary is Godot
//! scenery, never the source of truth.

use godot::builtin::{
    Array, Color, GString, PackedColorArray, PackedFloat32Array, PackedInt32Array,
    PackedVector2Array, PackedVector3Array, VarArray, VarDictionary, Vector2, Vector3, Vector3i,
};
use godot::classes::mesh::{ArrayCustomFormat, ArrayFormat, ArrayType, PrimitiveType};
use godot::classes::{ArrayMesh, INode, Node};
use godot::obj::{EngineBitfield, EngineEnum};
use godot::prelude::*;

use keep_core::command::{
    AuthorId, CommandOp, MAX_TOWER_HEIGHT_MM, MAX_TOWER_RADIUS_MM, MAX_WALL_HEIGHT_MM,
    MAX_WALL_THICKNESS_MM, MIN_TOWER_HEIGHT_MM, MIN_TOWER_RADIUS_MM, MIN_WALL_HEIGHT_MM,
    MIN_WALL_THICKNESS_MM,
};
use keep_core::document::{CastleDocument, Element};
use keep_core::mesh::{ChunkMeshData, SceneData};
use keep_core::picking::SnapKind;
use keep_core::session::{Session, DEFAULT_SITE_ID};
use keep_core::terrain::{SiteTerrain, TerrainMeshData};

/// The only class the game needs for M0a/M0b.
#[derive(GodotClass)]
#[class(base = Node)]
pub struct KeepBridge {
    base: Base<Node>,
    /// Host-authoritative session: document, command log and undo live here.
    session: Session,
    /// Author used for every local interactive edit.
    player_author: AuthorId,
}

#[godot_api]
impl INode for KeepBridge {
    fn init(base: Base<Node>) -> Self {
        Self {
            base,
            session: Session::from_document(
                DEFAULT_SITE_ID,
                SiteTerrain::m0a_promontory(),
                keep_core::fixtures::m0b_showcase_document(),
            ),
            player_author: AuthorId::player("local")
                .expect("built-in local player author id is always valid"),
        }
    }
}

#[godot_api]
impl KeepBridge {
    /// Build the fixed M0a look scene (one battered wall, one round tower).
    #[func]
    fn build_m0a_showcase(&mut self) -> VarDictionary {
        self.session = Session::from_document(
            DEFAULT_SITE_ID,
            SiteTerrain::m0a_promontory(),
            keep_core::fixtures::m0a_showcase_document(),
        );
        self.session_scene()
    }

    /// Build the M0b junction and void look scene.
    #[func]
    fn build_m0b_showcase(&mut self) -> VarDictionary {
        self.session = Session::from_document(
            DEFAULT_SITE_ID,
            SiteTerrain::m0a_promontory(),
            keep_core::fixtures::m0b_showcase_document(),
        );
        self.session_scene()
    }

    /// Build the large concentric-castle stress scene (~200k blocks).
    #[func]
    fn build_stress_200k(&mut self) -> VarDictionary {
        self.session = Session::from_document(
            DEFAULT_SITE_ID,
            SiteTerrain::m0a_promontory(),
            keep_core::fixtures::stress_200k_document(),
        );
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
                self.session = session;
                let mut result = self.session_scene();
                result.set("ok", true);
                result
            }
            Err(error) => error_dictionary(&error),
        }
    }
}

impl KeepBridge {
    /// Rebuild the current session document into a scene dictionary.
    fn session_scene(&self) -> VarDictionary {
        let scene = self.session.build_scene();
        let mut result = scene_to_dictionary(
            scene,
            self.session.document(),
            self.session.terrain(),
            self.session.document().document_hash(),
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
    result.set("chunks", &chunks);
    result.set("elements", &elements_array(document, terrain));
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
        }
    }
    elements
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
