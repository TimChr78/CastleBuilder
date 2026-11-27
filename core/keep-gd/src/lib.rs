//! Thin GDExtension binding: commands in, mesh buffers out.
//!
//! The bridge owns no authoritative castle state. Each `build_*` call runs the
//! deterministic `keep-core` pipeline and returns Godot resources describing
//! the terrain and merged chunk LOD meshes.

use godot::builtin::{
    Array, Color, GString, PackedColorArray, PackedFloat32Array, PackedInt32Array,
    PackedVector2Array, PackedVector3Array, VarArray, VarDictionary, Vector2, Vector3, Vector3i,
};
use godot::classes::mesh::{ArrayCustomFormat, ArrayFormat, ArrayType, PrimitiveType};
use godot::classes::{ArrayMesh, INode, Node};
use godot::obj::{EngineBitfield, EngineEnum};
use godot::prelude::*;

use keep_core::mesh::{ChunkMeshData, SceneData};
use keep_core::terrain::TerrainMeshData;

/// The only class the game needs for M0a.
#[derive(GodotClass)]
#[class(base = Node)]
pub struct KeepBridge {
    base: Base<Node>,
}

#[godot_api]
impl INode for KeepBridge {
    fn init(base: Base<Node>) -> Self {
        Self { base }
    }
}

#[godot_api]
impl KeepBridge {
    /// Build the fixed M0a look scene (one battered wall, one round tower).
    #[func]
    fn build_m0a_showcase(&self) -> VarDictionary {
        scene_to_dictionary(keep_core::fixtures::m0a_showcase())
    }

    /// Build the large concentric-castle stress scene (~200k blocks).
    #[func]
    fn build_stress_200k(&self) -> VarDictionary {
        scene_to_dictionary(keep_core::fixtures::stress_200k())
    }
}

fn scene_to_dictionary(scene: SceneData) -> VarDictionary {
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
