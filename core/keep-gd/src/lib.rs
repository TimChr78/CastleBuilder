//! Thin GDExtension binding: commands in, mesh buffers out.
//!
//! `KeepBridge` is a thin *session* bridge, not an authority: it keeps the
//! active [`CastleDocument`] and [`SiteTerrain`] so interactive edits can be
//! applied and re-meshed, but every rebuild still runs the deterministic
//! `keep-core` pipeline. The returned dictionary is Godot scenery, never the
//! source of truth.

use godot::builtin::{
    Array, Color, GString, PackedColorArray, PackedFloat32Array, PackedInt32Array,
    PackedVector2Array, PackedVector3Array, VarArray, VarDictionary, Vector2, Vector3, Vector3i,
};
use godot::classes::mesh::{ArrayCustomFormat, ArrayFormat, ArrayType, PrimitiveType};
use godot::classes::{ArrayMesh, INode, Node};
use godot::obj::{EngineBitfield, EngineEnum};
use godot::prelude::*;

use keep_core::document::{BatteredWall, CastleDocument, RoundTower};
use keep_core::mesh::{build_scene, ChunkMeshData, SceneData};
use keep_core::picking::SnapKind;
use keep_core::terrain::{SiteTerrain, TerrainMeshData};

/// First id handed to an interactively added element. Fixture ids stay well
/// below this, so interactive edits never collide with the loaded showcase.
const FIRST_INTERACTIVE_ID: u64 = 1_000;

/// The only class the game needs for M0a/M0b.
#[derive(GodotClass)]
#[class(base = Node)]
pub struct KeepBridge {
    base: Base<Node>,
    terrain: SiteTerrain,
    document: CastleDocument,
    next_id: u64,
    /// Ids of interactively added elements, newest last, for undo.
    history: Vec<u64>,
}

#[godot_api]
impl INode for KeepBridge {
    fn init(base: Base<Node>) -> Self {
        Self {
            base,
            terrain: SiteTerrain::m0a_promontory(),
            document: keep_core::fixtures::m0b_showcase_document(),
            next_id: FIRST_INTERACTIVE_ID,
            history: Vec::new(),
        }
    }
}

#[godot_api]
impl KeepBridge {
    /// Build the fixed M0a look scene (one battered wall, one round tower).
    #[func]
    fn build_m0a_showcase(&mut self) -> VarDictionary {
        self.load_document(keep_core::fixtures::m0a_showcase_document())
    }

    /// Build the M0b junction and void look scene.
    #[func]
    fn build_m0b_showcase(&mut self) -> VarDictionary {
        self.load_document(keep_core::fixtures::m0b_showcase_document())
    }

    /// Build the large concentric-castle stress scene (~200k blocks).
    #[func]
    fn build_stress_200k(&mut self) -> VarDictionary {
        self.load_document(keep_core::fixtures::stress_200k_document())
    }

    /// Pick the promontory ground under a world-space ray. `point` is in metres.
    #[func]
    fn pick_terrain(&self, ray_origin: Vector3, ray_dir: Vector3) -> VarDictionary {
        let hit = keep_core::picking::pick_terrain(
            &self.terrain,
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
        let snap = keep_core::picking::snap_point(&self.document, point_mm, radius_mm);

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
    #[func]
    fn add_wall(
        &mut self,
        start_xz: Vector2,
        end_xz: Vector2,
        height_m: f32,
        thickness_m: f32,
    ) -> VarDictionary {
        let id = self.take_next_id();
        let height_mm = quantise_m(height_m).max(1_000);
        let thickness_mm = quantise_m(thickness_m).max(200);
        let wall = BatteredWall::new(
            id,
            format!("Wall {id}"),
            quantise_xz(start_xz),
            quantise_xz(end_xz),
            height_mm,
            thickness_mm,
        );
        self.document.add_wall(wall);
        self.history.push(id);
        self.rebuild()
    }

    /// Add a round tower and return the rebuilt scene.
    #[func]
    fn add_tower(&mut self, center_xz: Vector2, radius_m: f32, height_m: f32) -> VarDictionary {
        let id = self.take_next_id();
        let radius_mm = quantise_m(radius_m).max(800);
        let height_mm = quantise_m(height_m).max(1_000);
        let tower = RoundTower::new(
            id,
            format!("Tower {id}"),
            quantise_xz(center_xz),
            radius_mm,
            height_mm,
        );
        self.document.add_tower(tower);
        self.history.push(id);
        self.rebuild()
    }

    /// Remove the most recently added interactive element, if any.
    #[func]
    fn undo_last(&mut self) -> VarDictionary {
        let undone = self.history.pop();
        if let Some(id) = undone {
            remove_element(&mut self.document, id);
        }
        let mut result = self.rebuild();
        result.set("undone", undone.is_some());
        result
    }
}

impl KeepBridge {
    /// Replace the session document, reset interactive ids and rebuild.
    fn load_document(&mut self, document: CastleDocument) -> VarDictionary {
        let scene = build_scene(&document, &self.terrain);
        let document_hash = document.document_hash();
        self.document = document;
        self.next_id = FIRST_INTERACTIVE_ID;
        self.history.clear();
        scene_to_dictionary(scene, document_hash)
    }

    /// Rebuild the current session document into a scene dictionary.
    fn rebuild(&self) -> VarDictionary {
        let scene = build_scene(&self.document, &self.terrain);
        scene_to_dictionary(scene, self.document.document_hash())
    }

    fn take_next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// `CastleDocument` intentionally exposes no removal API (deletion is a command
/// the core will own later), so undo rebuilds the document without `id`.
fn remove_element(document: &mut CastleDocument, id: u64) {
    let mut rebuilt = CastleDocument::new();
    for element in document.elements() {
        if element.id() != id {
            rebuilt.insert(element.clone());
        }
    }
    for opening in document.openings() {
        rebuilt.insert_opening(opening.clone());
    }
    *document = rebuilt;
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

fn scene_to_dictionary(scene: SceneData, document_hash: u64) -> VarDictionary {
    let mut result = VarDictionary::new();

    let terrain = terrain_mesh(&scene.terrain_mesh);
    result.set("terrain", &terrain);

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
    result.set("block_count", scene.block_count as i64);
    result.set("chunk_count", scene.chunks.len() as i64);
    result.set("triangle_count", scene.triangle_count as i64);
    let layout_hash = GString::from(&format!("{:016x}", scene.layout_hash));
    result.set("layout_hash", &layout_hash);
    let document_hash = GString::from(&format!("{document_hash:016x}"));
    result.set("document_hash", &document_hash);
    result
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
