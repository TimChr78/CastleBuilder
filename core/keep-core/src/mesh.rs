//! Merged chunk meshing and baked sky/hemisphere ambient occlusion.
//!
//! Ordinary stone blocks emit only their exposed faces (two quads, four
//! triangles for a mid-course wall stone; up to four faces for top and end
//! stones), staying well below a full 12-triangle cube. Bevels are shader work
//! driven by the per-vertex local face UVs in `[0, 1]`.
//!
//! Blocks are bucketed into 16 m spatial chunks. Each chunk produces:
//! * `lod0`: full exposed-face blocks;
//! * `lod1`: a lofted shell per element in the chunk, following the tower arc
//!   or the wall's oriented battered footprint;
//! * `lod2`: a single coarse massing proxy for the chunk.
//!
//! Vertex attributes follow the M0a contract:
//! * `positions` / `normals` / `uvs`;
//! * `colors`: `[ao, tint, age, jitter_seed]`;
//! * `custom0`: `[build_index_norm, course_index, block_width_m, block_height_m]`.
//!   Roof blocks store a *negative* height so the shader can identify slate
//!   strips explicitly; the absolute value is always the real height.

use std::collections::BTreeMap;

use crate::document::{
    BatteredWall, CastleDocument, Element, ElementKind, Gatehouse, RoundTower, TowerPlan,
};
use crate::masonry::{generate_blocks, Block, BlockFace, BlockRole};
use crate::terrain::SiteTerrain;
use crate::{fnv1a_64, fnv1a_64_update, GENERATOR_VERSION};
use libm::{atan2, cos, round, sin, sqrt, trunc};

/// Chunk edge length in millimetres (16 m).
pub const CHUNK_MM: i64 = 16_000;

#[derive(Clone, Debug, Default)]
pub struct ChunkMeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// `[baked_ao, tint, age, jitter_seed]`
    pub colors: Vec<[f32; 4]>,
    /// `[build_index_norm, course_index, block_width_m, block_height_m]`
    /// where a negative height marks a [`BlockRole::Roof`] block.
    pub custom0: Vec<[f32; 4]>,
    pub indices: Vec<i32>,
}

impl ChunkMeshData {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }
}

#[derive(Clone, Debug)]
pub struct ChunkMesh {
    /// Integer chunk key (`Vector3i` on the Godot side).
    pub key: [i32; 3],
    /// Block-centroid centre in metres, used for LOD and camera distance.
    pub centre: [f32; 3],
    pub block_count: usize,
    pub lod0: ChunkMeshData,
    pub lod1: ChunkMeshData,
    pub lod2: ChunkMeshData,
}

/// Everything needed to display one fixture: terrain plus merged chunk meshes.
#[derive(Clone, Debug)]
pub struct SceneData {
    pub terrain_mesh: crate::terrain::TerrainMeshData,
    pub chunks: Vec<ChunkMesh>,
    pub block_count: usize,
    pub triangle_count: usize,
    pub layout_hash: u64,
}

impl SceneData {
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

/// Generate blocks, then bucket and mesh them into merged chunk meshes.
///
/// Ditches are carved into a private copy of the terrain before both masonry
/// grounding and the terrain mesh are generated, so `terrain_mesh()` always
/// displays the earthworks described by the document.
pub fn build_scene(document: &CastleDocument, terrain: &SiteTerrain) -> SceneData {
    let ditches: Vec<&crate::document::Ditch> = document.ditches().collect();
    let sculpts: Vec<&crate::document::TerrainSculpt> = document.terrain_sculpts().collect();
    let effective_terrain = terrain.with_earthworks(&ditches, &sculpts);
    let terrain = &effective_terrain;

    let blocks = generate_blocks(document, terrain);
    let total_blocks = blocks.len();
    let layout_hash = layout_hash_of(&blocks);
    let occupancy = Occupancy::build(&blocks, terrain);

    let mut buckets: BTreeMap<[i32; 3], Vec<Block>> = BTreeMap::new();
    for block in blocks {
        let centre = block.centre_mm();
        let key = [
            div_euclid(centre[0].round() as i64, CHUNK_MM),
            div_euclid(centre[1].round() as i64, CHUNK_MM),
            div_euclid(centre[2].round() as i64, CHUNK_MM),
        ];
        buckets.entry(key).or_default().push(block);
    }

    let mut chunks = Vec::with_capacity(buckets.len());
    let mut triangle_count = 0usize;
    for (key, chunk_blocks) in buckets {
        let chunk = build_chunk(
            key,
            &chunk_blocks,
            total_blocks,
            terrain,
            &occupancy,
            document,
        );
        triangle_count += chunk.lod0.triangle_count();
        chunks.push(chunk);
    }

    SceneData {
        terrain_mesh: terrain.terrain_mesh(),
        chunks,
        block_count: total_blocks,
        triangle_count,
        layout_hash,
    }
}

/// Hash the generated geometry and its vertex attributes. This is the layout
/// hash half of the co-play divergence check: matching documents with a
/// mismatched layout hash means generator divergence, not a lost command.
fn layout_hash_of(blocks: &[Block]) -> u64 {
    let mut hash = fnv1a_64(b"keepwright-layout-v1");
    hash = fnv1a_64_update(hash, &GENERATOR_VERSION.to_le_bytes());
    hash = fnv1a_64_update(hash, &(blocks.len() as u64).to_le_bytes());
    for block in blocks {
        hash = fnv1a_64_update(hash, &[block.kind as u8]);
        hash = fnv1a_64_update(hash, &block.element_id.to_le_bytes());
        hash = fnv1a_64_update(hash, &block.course_index.to_le_bytes());
        hash = fnv1a_64_update(hash, &block.run_pos_mm.to_le_bytes());
        hash = fnv1a_64_update(hash, &block.build_index.to_le_bytes());
        for corner in block.corners_mm {
            for value in corner {
                hash = fnv1a_64_update(hash, &value.to_le_bytes());
            }
        }
        hash = fnv1a_64_update(hash, &block.block_width_mm.to_le_bytes());
        hash = fnv1a_64_update(hash, &block.block_height_mm.to_le_bytes());
        hash = fnv1a_64_update(hash, &block.tint.to_bits().to_le_bytes());
        hash = fnv1a_64_update(hash, &block.age.to_bits().to_le_bytes());
        hash = fnv1a_64_update(hash, &block.jitter_seed.to_bits().to_le_bytes());
        for exposed in block.expose {
            hash = fnv1a_64_update(hash, &[u8::from(exposed)]);
        }
        hash = fnv1a_64_update(hash, &[block.role as u8]);
    }
    hash
}

fn build_chunk(
    key: [i32; 3],
    blocks: &[Block],
    total_blocks: usize,
    terrain: &SiteTerrain,
    occupancy: &Occupancy,
    document: &CastleDocument,
) -> ChunkMesh {
    let mut lod0 = ChunkMeshData::default();
    let mut chunk_bounds = Aabb::empty();
    let mut centre_sum = [0.0f64; 3];
    let mut course_sum = 0.0f64;
    let mut build_sum = 0.0f64;
    let mut element_blocks: BTreeMap<u64, Vec<&Block>> = BTreeMap::new();

    for block in blocks {
        let centre = block.centre_mm();
        centre_sum[0] += centre[0];
        centre_sum[1] += centre[1];
        centre_sum[2] += centre[2];
        course_sum += f64::from(block.course_index);
        build_sum += block.build_index as f64;
        chunk_bounds.include(&Aabb::from_block(block));
        element_blocks
            .entry(block.element_id)
            .or_default()
            .push(block);

        let build_norm = if total_blocks > 1 {
            block.build_index as f32 / (total_blocks - 1) as f32
        } else {
            0.0
        };
        emit_block(&mut lod0, block, build_norm, terrain, occupancy);
    }

    let count = blocks.len().max(1) as f64;
    let centre = [
        (centre_sum[0] / count / 1000.0) as f32,
        (centre_sum[1] / count / 1000.0) as f32,
        (centre_sum[2] / count / 1000.0) as f32,
    ];
    let avg_course = (course_sum / count) as f32;
    let avg_build = if total_blocks > 1 {
        ((build_sum / count) / (total_blocks - 1) as f64) as f32
    } else {
        0.0
    };

    let mut lod1 = ChunkMeshData::default();
    for (element_id, element_chunk_blocks) in &element_blocks {
        let Some(element) = document.get(*element_id) else {
            continue;
        };
        let params = EmitParams {
            tint: 0.92,
            age: 0.8,
            jitter_seed: *element_id as f32 * 1e-9,
            build_norm: avg_build,
            course: avg_course,
            width: 1.0,
            height: 1.0,
        };
        match element {
            Element::BatteredWall(wall) => emit_lod1_wall(
                &mut lod1,
                wall,
                element_chunk_blocks,
                &params,
                terrain,
                occupancy,
            ),
            Element::RoundTower(tower) => {
                if tower.plan == TowerPlan::Round {
                    emit_lod1_tower(
                        &mut lod1,
                        tower,
                        element_chunk_blocks,
                        &params,
                        terrain,
                        occupancy,
                    );
                } else {
                    emit_lod1_shaped_tower(
                        &mut lod1,
                        tower,
                        element_chunk_blocks,
                        &params,
                        terrain,
                        occupancy,
                    );
                }
            }
            Element::Gatehouse(gatehouse) => emit_lod1_gatehouse(
                &mut lod1,
                gatehouse,
                element_chunk_blocks,
                &params,
                terrain,
                occupancy,
            ),
            Element::Keep(_) | Element::Range(_) => {
                let mut aabb = Aabb::empty();
                for block in element_chunk_blocks {
                    aabb.include(&Aabb::from_block(block));
                }
                if !aabb.is_empty() {
                    emit_box(&mut lod1, &aabb, &params, terrain, occupancy);
                }
            }
            Element::Ditch(_) | Element::TerrainSculpt(_) => {}
        }
    }

    let mut lod2 = ChunkMeshData::default();
    if !chunk_bounds.is_empty() {
        let params = EmitParams {
            tint: 0.9,
            age: 0.75,
            jitter_seed: key[0] as f32 * 0.01 + key[2] as f32 * 0.001,
            build_norm: avg_build,
            course: avg_course,
            width: (chunk_bounds.max[0] - chunk_bounds.min[0]) as f32 / 1000.0,
            height: (chunk_bounds.max[1] - chunk_bounds.min[1]) as f32 / 1000.0,
        };
        emit_box(&mut lod2, &chunk_bounds, &params, terrain, occupancy);
    }

    ChunkMesh {
        key,
        centre,
        block_count: blocks.len(),
        lod0,
        lod1,
        lod2,
    }
}

#[derive(Clone, Copy)]
struct EmitParams {
    tint: f32,
    age: f32,
    jitter_seed: f32,
    build_norm: f32,
    course: f32,
    width: f32,
    height: f32,
}

impl EmitParams {
    fn custom(self) -> [f32; 4] {
        [self.build_norm, self.course, self.width, self.height]
    }
}

fn emit_block(
    data: &mut ChunkMeshData,
    block: &Block,
    build_norm: f32,
    terrain: &SiteTerrain,
    occupancy: &Occupancy,
) {
    // Explicit roof tag: the sign of `custom0[3]` says "slate", never the
    // block's size. A thin brick or gatehouse course must stay stone.
    let block_height = block.block_height_mm as f32 / 1000.0;
    let encoded_height = if block.role == BlockRole::Roof {
        -block_height
    } else {
        block_height
    };
    let params = EmitParams {
        tint: block.tint,
        age: block.age,
        jitter_seed: block.jitter_seed,
        build_norm,
        course: block.course_index as f32,
        width: block.block_width_mm as f32 / 1000.0,
        height: encoded_height,
    };

    for face in [
        BlockFace::Bottom,
        BlockFace::Top,
        BlockFace::Front,
        BlockFace::Back,
        BlockFace::End0,
        BlockFace::End1,
    ] {
        if !block.face_enabled(face) {
            continue;
        }
        let indices = Block::face_corners(face);
        let mut corners = [[0.0f32; 3]; 4];
        for (slot, corner_index) in indices.iter().enumerate() {
            let c = block.corners_mm[*corner_index];
            corners[slot] = [
                c[0] as f32 / 1000.0,
                c[1] as f32 / 1000.0,
                c[2] as f32 / 1000.0,
            ];
        }
        emit_quad(data, corners, params, terrain, occupancy);
    }
}

/// LOD1 wall: a battered prism following the wall's oriented footprint and the
/// actual along-run span of this chunk, not an axis-aligned box.
fn emit_lod1_wall(
    data: &mut ChunkMeshData,
    wall: &BatteredWall,
    blocks: &[&Block],
    params: &EmitParams,
    terrain: &SiteTerrain,
    occupancy: &Occupancy,
) {
    let dx = (wall.end_mm[0] - wall.start_mm[0]) as f64;
    let dz = (wall.end_mm[1] - wall.start_mm[1]) as f64;
    let len = sqrt(dx * dx + dz * dz);
    if len < 1.0 {
        return;
    }
    let dir = [dx / len, dz / len];
    let right = [-dir[1], dir[0]];
    let base_x = wall.start_mm[0] as f64;
    let base_z = wall.start_mm[1] as f64;

    let mut a_min = i64::MAX;
    let mut a_max = i64::MIN;
    let mut y_min = i64::MAX;
    let mut y_max = i64::MIN;
    for block in blocks {
        a_min = a_min.min(block.run_pos_mm);
        a_max = a_max.max(block.run_pos_mm + block.block_width_mm);
        for corner in block.corners_mm {
            y_min = y_min.min(corner[1]);
            y_max = y_max.max(corner[1]);
        }
    }
    if a_max <= a_min || y_max <= y_min {
        return;
    }

    let mut d_bottom = (f64::MAX, f64::MIN);
    let mut d_top = (f64::MAX, f64::MIN);
    for block in blocks {
        for corner in block.corners_mm {
            let d = (corner[0] as f64 - base_x) * right[0] + (corner[2] as f64 - base_z) * right[1];
            if corner[1] <= y_min {
                d_bottom.0 = d_bottom.0.min(d);
                d_bottom.1 = d_bottom.1.max(d);
            }
            if corner[1] >= y_max {
                d_top.0 = d_top.0.min(d);
                d_top.1 = d_top.1.max(d);
            }
        }
    }
    if d_bottom.0 > d_bottom.1 || d_top.0 > d_top.1 {
        return;
    }

    let point = |a: i64, d: f64, y: i64| -> [i64; 3] {
        let x = base_x + dir[0] * a as f64 + right[0] * d;
        let z = base_z + dir[1] * a as f64 + right[1] * d;
        [round(x) as i64, y, round(z) as i64]
    };
    let corners = [
        point(a_min, d_bottom.0, y_min),
        point(a_max, d_bottom.0, y_min),
        point(a_max, d_bottom.1, y_min),
        point(a_min, d_bottom.1, y_min),
        point(a_min, d_top.0, y_max),
        point(a_max, d_top.0, y_max),
        point(a_max, d_top.1, y_max),
        point(a_min, d_top.1, y_max),
    ];

    for face in [
        BlockFace::Front,
        BlockFace::Back,
        BlockFace::Top,
        BlockFace::End0,
        BlockFace::End1,
    ] {
        emit_corner_face(data, &corners, face, *params, terrain, occupancy);
    }
}

/// LOD1 tower: a segmented, battered cylindrical shell following the tower's
/// actual arc within this chunk.
fn emit_lod1_tower(
    data: &mut ChunkMeshData,
    tower: &RoundTower,
    blocks: &[&Block],
    params: &EmitParams,
    terrain: &SiteTerrain,
    occupancy: &Occupancy,
) {
    if blocks.is_empty() {
        return;
    }
    let cx = tower.centre_mm[0] as f64;
    let cz = tower.centre_mm[1] as f64;
    let radius = tower.radius_mm.max(500) as f64;

    let mut y_min = i64::MAX;
    let mut y_max = i64::MIN;
    for block in blocks {
        for corner in block.corners_mm {
            y_min = y_min.min(corner[1]);
            y_max = y_max.max(corner[1]);
        }
    }
    if y_max <= y_min {
        return;
    }

    let radius_of = |point: [i64; 3]| -> f64 {
        let dx = point[0] as f64 - cx;
        let dz = point[2] as f64 - cz;
        sqrt(dx * dx + dz * dz)
    };

    let mut angles = Vec::with_capacity(blocks.len());
    let mut half_step = 0.0f64;
    let mut outer_bottom = 0.0f64;
    let mut outer_top = 0.0f64;
    let mut inner_bottom = f64::MAX;
    let mut inner_top = f64::MAX;
    for block in blocks {
        let centre = block.centre_mm();
        angles.push(atan2(centre[2] - cz, centre[0] - cx));
        half_step = half_step.max(block.block_width_mm as f64 / (2.0 * radius));

        let block_outer_bottom = radius_of(block.corners_mm[0]).max(radius_of(block.corners_mm[1]));
        let block_outer_top = radius_of(block.corners_mm[4]).max(radius_of(block.corners_mm[5]));
        let block_inner_bottom = radius_of(block.corners_mm[2]).min(radius_of(block.corners_mm[3]));
        let block_inner_top = radius_of(block.corners_mm[6]).min(radius_of(block.corners_mm[7]));

        if block.corners_mm[0][1] <= y_min {
            outer_bottom = outer_bottom.max(block_outer_bottom);
            inner_bottom = inner_bottom.min(block_inner_bottom);
        }
        if block.corners_mm[4][1] >= y_max {
            outer_top = outer_top.max(block_outer_top);
            inner_top = inner_top.min(block_inner_top);
        }
    }
    if inner_bottom == f64::MAX || inner_top == f64::MAX {
        return;
    }

    // Unwrap the arc around the first stone so chunks near the +/-pi seam still
    // produce one contiguous shell.
    let reference = angles[0];
    for angle in &mut angles {
        while *angle - reference > std::f64::consts::PI {
            *angle -= std::f64::consts::TAU;
        }
        while *angle - reference < -std::f64::consts::PI {
            *angle += std::f64::consts::TAU;
        }
    }
    let mut a_min = f64::INFINITY;
    let mut a_max = f64::NEG_INFINITY;
    for angle in &angles {
        a_min = a_min.min(*angle);
        a_max = a_max.max(*angle);
    }
    a_min -= half_step;
    a_max += half_step;
    if a_max <= a_min {
        return;
    }

    let segment_angle = std::f64::consts::TAU / 24.0;
    let segments = (((a_max - a_min) / segment_angle).ceil() as usize).clamp(2, 48);
    let point = |angle: f64, r: f64, y: i64| -> [i64; 3] {
        [
            round(cx + cos(angle) * r) as i64,
            y,
            round(cz + sin(angle) * r) as i64,
        ]
    };

    for segment in 0..segments {
        let t0 = segment as f64 / segments as f64;
        let t1 = (segment + 1) as f64 / segments as f64;
        let a0 = a_min + (a_max - a_min) * t0;
        let a1 = a_min + (a_max - a_min) * t1;

        let ob0 = point(a0, outer_bottom, y_min);
        let ob1 = point(a1, outer_bottom, y_min);
        let ot0 = point(a0, outer_top, y_max);
        let ot1 = point(a1, outer_top, y_max);
        let ib0 = point(a0, inner_bottom, y_min);
        let ib1 = point(a1, inner_bottom, y_min);
        let it0 = point(a0, inner_top, y_max);
        let it1 = point(a1, inner_top, y_max);

        // Outward-facing outer surface, inner surface facing the hollow, and
        // the top ring cap. Winding follows the right-hand outward normal.
        emit_metre_quad(data, [ob1, ob0, ot0, ot1], *params, terrain, occupancy);
        emit_metre_quad(data, [ib0, ib1, it1, it0], *params, terrain, occupancy);
        emit_metre_quad(data, [ot1, ot0, it0, it1], *params, terrain, occupancy);
    }

    // Close the shell at the two angular ends of this chunk.
    let ob = point(a_min, outer_bottom, y_min);
    let ot = point(a_min, outer_top, y_max);
    let ib = point(a_min, inner_bottom, y_min);
    let it = point(a_min, inner_top, y_max);
    emit_metre_quad(data, [ot, ob, ib, it], *params, terrain, occupancy);

    let ob = point(a_max, outer_bottom, y_min);
    let ot = point(a_max, outer_top, y_max);
    let ib = point(a_max, inner_bottom, y_min);
    let it = point(a_max, inner_top, y_max);
    emit_metre_quad(data, [ob, ot, it, ib], *params, terrain, occupancy);
}

/// LOD1 for shaped (square / D-shaped) towers: an oriented, battered prism
/// following the local footprint of this chunk, instead of a round cylinder.
fn emit_lod1_shaped_tower(
    data: &mut ChunkMeshData,
    tower: &RoundTower,
    blocks: &[&Block],
    params: &EmitParams,
    terrain: &SiteTerrain,
    occupancy: &Occupancy,
) {
    if blocks.is_empty() {
        return;
    }
    let orientation = tower.orientation_deg as f64 * std::f64::consts::PI / 180.0;
    let (sin_o, cos_o) = (sin(orientation), cos(orientation));
    let cx = tower.centre_mm[0] as f64;
    let cz = tower.centre_mm[1] as f64;

    let local = |point: [i64; 3]| -> (f64, f64) {
        let dx = point[0] as f64 - cx;
        let dz = point[2] as f64 - cz;
        (dx * cos_o + dz * sin_o, -dx * sin_o + dz * cos_o)
    };

    let mut y_min = i64::MAX;
    let mut y_max = i64::MIN;
    for block in blocks {
        for corner in block.corners_mm {
            y_min = y_min.min(corner[1]);
            y_max = y_max.max(corner[1]);
        }
    }
    if y_max <= y_min {
        return;
    }

    let mut bottom = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    let mut top = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for block in blocks {
        for corner in block.corners_mm {
            let (u, v) = local(corner);
            let range = if corner[1] <= y_min {
                &mut bottom
            } else if corner[1] >= y_max {
                &mut top
            } else {
                continue;
            };
            range.0 = range.0.min(u);
            range.1 = range.1.max(u);
            range.2 = range.2.min(v);
            range.3 = range.3.max(v);
        }
    }
    if bottom.0 > bottom.1 || bottom.2 > bottom.3 || top.0 > top.1 || top.2 > top.3 {
        return;
    }

    let point = |u: f64, v: f64, y: i64| -> [i64; 3] {
        [
            round(cx + u * cos_o - v * sin_o) as i64,
            y,
            round(cz + u * sin_o + v * cos_o) as i64,
        ]
    };
    let corners = [
        point(bottom.0, bottom.2, y_min),
        point(bottom.1, bottom.2, y_min),
        point(bottom.1, bottom.3, y_min),
        point(bottom.0, bottom.3, y_min),
        point(top.0, top.2, y_max),
        point(top.1, top.2, y_max),
        point(top.1, top.3, y_max),
        point(top.0, top.3, y_max),
    ];

    for face in [
        BlockFace::Front,
        BlockFace::Back,
        BlockFace::Top,
        BlockFace::End0,
        BlockFace::End1,
    ] {
        emit_corner_face(data, &corners, face, *params, terrain, occupancy);
    }
}

/// LOD1 for a gatehouse: two flanking battered drums plus an oriented bridge
/// prism that begins *above* the passage, so the gate is visibly open at LOD1.
fn emit_lod1_gatehouse(
    data: &mut ChunkMeshData,
    gatehouse: &Gatehouse,
    blocks: &[&Block],
    params: &EmitParams,
    terrain: &SiteTerrain,
    occupancy: &Occupancy,
) {
    let towers = gatehouse.flank_towers();
    let tower_blocks: Vec<&Block> = blocks
        .iter()
        .copied()
        .filter(|block| block.kind == ElementKind::RoundTower)
        .collect();
    for (index, tower) in towers.iter().enumerate() {
        let other = towers[1 - index].centre_mm;
        let own: Vec<&Block> = tower_blocks
            .iter()
            .copied()
            .filter(|block| {
                let centre = block.centre_mm();
                let own_dx = centre[0] - tower.centre_mm[0] as f64;
                let own_dz = centre[2] - tower.centre_mm[1] as f64;
                let other_dx = centre[0] - other[0] as f64;
                let other_dz = centre[2] - other[1] as f64;
                let own_distance = sqrt(own_dx * own_dx + own_dz * own_dz);
                let other_distance = sqrt(other_dx * other_dx + other_dz * other_dz);
                own_distance <= other_distance && own_distance <= tower.radius_mm as f64 + 2_000.0
            })
            .collect();
        if !own.is_empty() {
            emit_lod1_tower(data, tower, &own, params, terrain, occupancy);
        }
    }

    let bridge: Vec<&Block> = blocks
        .iter()
        .copied()
        .filter(|block| block.kind == ElementKind::Gatehouse)
        .collect();
    if bridge.is_empty() {
        return;
    }

    let ground = terrain.height_mm(gatehouse.centre_mm[0], gatehouse.centre_mm[1]);
    let y_min = ground + gatehouse.passage_height_mm.max(1);
    let dir = gatehouse.direction();
    let right = [-dir[1], dir[0]];
    let cx = gatehouse.centre_mm[0] as f64;
    let cz = gatehouse.centre_mm[1] as f64;

    let mut a_min = f64::MAX;
    let mut a_max = f64::MIN;
    let mut d_min = f64::MAX;
    let mut d_max = f64::MIN;
    let mut y_max = i64::MIN;
    for block in &bridge {
        for corner in block.corners_mm {
            let dx = corner[0] as f64 - cx;
            let dz = corner[2] as f64 - cz;
            let a = dx * dir[0] + dz * dir[1];
            let d = dx * right[0] + dz * right[1];
            a_min = a_min.min(a);
            a_max = a_max.max(a);
            d_min = d_min.min(d);
            d_max = d_max.max(d);
            y_max = y_max.max(corner[1]);
        }
    }
    if a_max <= a_min || d_max <= d_min || y_max <= y_min {
        return;
    }

    let point = |a: f64, d: f64, y: i64| -> [i64; 3] {
        [
            round(cx + dir[0] * a + right[0] * d) as i64,
            y,
            round(cz + dir[1] * a + right[1] * d) as i64,
        ]
    };
    let corners = [
        point(a_min, d_min, y_min),
        point(a_max, d_min, y_min),
        point(a_max, d_max, y_min),
        point(a_min, d_max, y_min),
        point(a_min, d_min, y_max),
        point(a_max, d_min, y_max),
        point(a_max, d_max, y_max),
        point(a_min, d_max, y_max),
    ];
    for face in [
        BlockFace::Front,
        BlockFace::Back,
        BlockFace::Top,
        BlockFace::End0,
        BlockFace::End1,
    ] {
        emit_corner_face(data, &corners, face, *params, terrain, occupancy);
    }
}

/// Coarse six-faced massing proxy used by LOD2.
fn emit_box(
    data: &mut ChunkMeshData,
    aabb: &Aabb,
    params: &EmitParams,
    terrain: &SiteTerrain,
    occupancy: &Occupancy,
) {
    let lo = [
        aabb.min[0] as f32 / 1000.0,
        aabb.min[1] as f32 / 1000.0,
        aabb.min[2] as f32 / 1000.0,
    ];
    let hi = [
        aabb.max[0] as f32 / 1000.0,
        aabb.max[1] as f32 / 1000.0,
        aabb.max[2] as f32 / 1000.0,
    ];
    let corners = [
        [lo[0], lo[1], lo[2]],
        [hi[0], lo[1], lo[2]],
        [hi[0], lo[1], hi[2]],
        [lo[0], lo[1], hi[2]],
        [lo[0], hi[1], lo[2]],
        [hi[0], hi[1], lo[2]],
        [hi[0], hi[1], hi[2]],
        [lo[0], hi[1], hi[2]],
    ];
    for face in [
        BlockFace::Bottom,
        BlockFace::Top,
        BlockFace::Front,
        BlockFace::Back,
        BlockFace::End0,
        BlockFace::End1,
    ] {
        emit_float_corner_face(data, &corners, face, *params, terrain, occupancy);
    }
}

fn emit_float_corner_face(
    data: &mut ChunkMeshData,
    corners: &[[f32; 3]; 8],
    face: BlockFace,
    params: EmitParams,
    terrain: &SiteTerrain,
    occupancy: &Occupancy,
) {
    let indices = Block::face_corners(face);
    let mut quad = [[0.0f32; 3]; 4];
    for (slot, corner_index) in indices.iter().enumerate() {
        quad[slot] = corners[*corner_index];
    }
    emit_quad(data, quad, params, terrain, occupancy);
}

fn emit_corner_face(
    data: &mut ChunkMeshData,
    corners: &[[i64; 3]; 8],
    face: BlockFace,
    params: EmitParams,
    terrain: &SiteTerrain,
    occupancy: &Occupancy,
) {
    let indices = Block::face_corners(face);
    let mut quad = [[0.0f32; 3]; 4];
    for (slot, corner_index) in indices.iter().enumerate() {
        let c = corners[*corner_index];
        quad[slot] = [
            c[0] as f32 / 1000.0,
            c[1] as f32 / 1000.0,
            c[2] as f32 / 1000.0,
        ];
    }
    emit_quad(data, quad, params, terrain, occupancy);
}

fn emit_metre_quad(
    data: &mut ChunkMeshData,
    corners_mm: [[i64; 3]; 4],
    params: EmitParams,
    terrain: &SiteTerrain,
    occupancy: &Occupancy,
) {
    let mut quad = [[0.0f32; 3]; 4];
    for (slot, c) in corners_mm.iter().enumerate() {
        quad[slot] = [
            c[0] as f32 / 1000.0,
            c[1] as f32 / 1000.0,
            c[2] as f32 / 1000.0,
        ];
    }
    emit_quad(data, quad, params, terrain, occupancy);
}

fn emit_quad(
    data: &mut ChunkMeshData,
    corners: [[f32; 3]; 4],
    params: EmitParams,
    terrain: &SiteTerrain,
    occupancy: &Occupancy,
) {
    let normal = quad_normal(&corners);
    let mut ao = [0.0f32; 4];
    for (slot, corner) in corners.iter().enumerate() {
        let sky = sky_visibility(*corner, normal, terrain, occupancy);
        ao[slot] = face_ao(sky, normal[1]);
    }
    let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    push_quad(
        data,
        QuadParams {
            corners,
            normal,
            uv,
            ao,
            tint: params.tint,
            age: params.age,
            jitter_seed: params.jitter_seed,
            custom: params.custom(),
        },
    );
}

struct QuadParams {
    corners: [[f32; 3]; 4],
    normal: [f32; 3],
    uv: [[f32; 2]; 4],
    ao: [f32; 4],
    tint: f32,
    age: f32,
    jitter_seed: f32,
    custom: [f32; 4],
}

fn push_quad(data: &mut ChunkMeshData, quad: QuadParams) {
    let base = data.positions.len() as i32;
    for slot in 0..4 {
        data.positions.push(quad.corners[slot]);
        data.normals.push(quad.normal);
        data.uvs.push(quad.uv[slot]);
        data.colors.push([
            quad.ao[slot].clamp(0.0, 1.0),
            quad.tint,
            quad.age,
            quad.jitter_seed,
        ]);
        data.custom0.push(quad.custom);
    }
    // Godot's front faces are clockwise. The corner order above gives a
    // right-hand normal pointing out of the stone, so reverse each triangle.
    data.indices
        .extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
}

fn quad_normal(corners: &[[f32; 3]; 4]) -> [f32; 3] {
    let e1 = sub(corners[1], corners[0]);
    let e2 = sub(corners[2], corners[0]);
    let n = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    normalize(n)
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(v: [f32; 3], factor: f32) -> [f32; 3] {
    [v[0] * factor, v[1] * factor, v[2] * factor]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = sqrt((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) as f64) as f32;
    if len <= 1.0e-6 {
        [0.0, 1.0, 0.0]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

fn div_euclid(value: i64, divisor: i64) -> i32 {
    value
        .div_euclid(divisor)
        .clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

#[derive(Clone, Debug)]
struct Aabb {
    min: [i64; 3],
    max: [i64; 3],
    empty: bool,
}

impl Aabb {
    fn empty() -> Self {
        Self {
            min: [i64::MAX; 3],
            max: [i64::MIN; 3],
            empty: true,
        }
    }

    fn from_block(block: &Block) -> Self {
        let mut aabb = Self::empty();
        for corner in block.corners_mm {
            aabb.include_point(corner);
        }
        aabb
    }

    fn is_empty(&self) -> bool {
        self.empty
    }

    fn include_point(&mut self, p: [i64; 3]) {
        for i in 0..3 {
            self.min[i] = self.min[i].min(p[i]);
            self.max[i] = self.max[i].max(p[i]);
        }
        self.empty = false;
    }

    fn include(&mut self, other: &Aabb) {
        if other.empty {
            return;
        }
        self.include_point(other.min);
        self.include_point(other.max);
    }
}

impl Default for Aabb {
    fn default() -> Self {
        Self::empty()
    }
}

/// Coarse castle-occupancy heightfield used for cheap sky occlusion.
///
/// Storing just the maximum stone height per cell is enough for hemisphere AO:
/// a ray point below the local masonry height is treated as blocked. This is
/// derived from the block bounds, so it follows the castle as it changes.
struct Occupancy {
    origin_mm: i64,
    cell_mm: i64,
    res: usize,
    heights_mm: Vec<i64>,
}

impl Occupancy {
    fn build(blocks: &[Block], terrain: &SiteTerrain) -> Self {
        let cell_mm = 500i64;
        let origin_mm = terrain.origin_mm();
        let res = ((terrain.extent_mm() - origin_mm).max(cell_mm) / cell_mm + 1) as usize;
        let mut heights_mm = vec![i64::MIN; res * res];

        for block in blocks {
            let mut min = [i64::MAX; 3];
            let mut max = [i64::MIN; 3];
            for corner in block.corners_mm {
                for i in 0..3 {
                    min[i] = min[i].min(corner[i]);
                    max[i] = max[i].max(corner[i]);
                }
            }
            let ix0 = ((min[0] - origin_mm) / cell_mm).clamp(0, res as i64 - 1) as usize;
            let ix1 = ((max[0] - origin_mm) / cell_mm).clamp(0, res as i64 - 1) as usize;
            let iz0 = ((min[2] - origin_mm) / cell_mm).clamp(0, res as i64 - 1) as usize;
            let iz1 = ((max[2] - origin_mm) / cell_mm).clamp(0, res as i64 - 1) as usize;
            let top = max[1];
            for iz in iz0..=iz1 {
                for ix in ix0..=ix1 {
                    let idx = iz * res + ix;
                    heights_mm[idx] = heights_mm[idx].max(top);
                }
            }
        }

        Self {
            origin_mm,
            cell_mm,
            res,
            heights_mm,
        }
    }

    fn cell(&self, x_m: f32, z_m: f32) -> (usize, usize) {
        let x_mm = (x_m as f64 * 1000.0) as i64;
        let z_mm = (z_m as f64 * 1000.0) as i64;
        let ix = ((x_mm - self.origin_mm) / self.cell_mm).clamp(0, self.res as i64 - 1) as usize;
        let iz = ((z_mm - self.origin_mm) / self.cell_mm).clamp(0, self.res as i64 - 1) as usize;
        (ix, iz)
    }

    fn height_mm(&self, x_mm: i64, z_mm: i64) -> i64 {
        let ix = ((x_mm - self.origin_mm) / self.cell_mm).clamp(0, self.res as i64 - 1) as usize;
        let iz = ((z_mm - self.origin_mm) / self.cell_mm).clamp(0, self.res as i64 - 1) as usize;
        self.heights_mm[iz * self.res + ix]
    }
}

/// Hemisphere sample directions in a local frame whose `+z` is the face normal
/// and whose `+y` is roughly world up. Each is normalised in `sky_visibility`.
const HEMI_DIRS: [[f32; 3]; 6] = [
    [0.0, 0.0, 1.0],
    [0.0, 0.78, 0.63],
    [0.68, 0.39, 0.62],
    [-0.68, 0.39, 0.62],
    [0.68, -0.39, 0.62],
    [-0.68, -0.39, 0.62],
];

const SKY_STEPS_M: [f32; 3] = [0.7, 1.8, 3.6];

/// Sky visibility for a surface point, with the hemisphere biased outward along
/// its face normal. The ray starts slightly off the surface and ignores the
/// point's own occupancy cell so a vertical wall does not shade itself.
fn sky_visibility(
    point: [f32; 3],
    normal: [f32; 3],
    terrain: &SiteTerrain,
    occupancy: &Occupancy,
) -> f32 {
    let n = normalize(normal);
    let up = [0.0, 1.0, 0.0];
    let up_dot = dot(n, up);
    let tangent = if up_dot.abs() > 0.99 {
        [1.0, 0.0, 0.0]
    } else {
        normalize(sub(up, scale(n, up_dot)))
    };
    let bitangent = cross(n, tangent);
    let origin_cell = occupancy.cell(point[0], point[2]);
    let start = add(add(point, scale(n, 0.35)), [0.0, 0.15, 0.0]);

    let mut visible = 0.0f32;
    for local in HEMI_DIRS {
        let dir = normalize(add(
            add(scale(bitangent, local[0]), scale(tangent, local[1])),
            scale(n, local[2]),
        ));
        let mut blocked = false;
        for step in SKY_STEPS_M {
            let sample = add(start, scale(dir, step));
            let x_mm = (sample[0] as f64 * 1000.0) as i64;
            let z_mm = (sample[2] as f64 * 1000.0) as i64;
            let y_mm = (sample[1] as f64 * 1000.0) as i64;
            let ground = terrain.height_mm(x_mm, z_mm);
            if y_mm <= ground {
                blocked = true;
                break;
            }
            if occupancy.cell(sample[0], sample[2]) == origin_cell {
                continue;
            }
            let castle = occupancy.height_mm(x_mm, z_mm);
            if castle != i64::MIN && y_mm < castle {
                blocked = true;
                break;
            }
        }
        if !blocked {
            visible += 1.0;
        }
    }
    visible / HEMI_DIRS.len() as f32
}

fn face_ao(sky: f32, normal_y: f32) -> f32 {
    let orientation = (0.5 + 0.5 * normal_y.clamp(-1.0, 1.0)).clamp(0.0, 1.0);
    ((0.16 + 0.84 * sky) * (0.60 + 0.40 * orientation)).clamp(0.0, 1.0)
}

/// Small helper used by tests and diagnostics.
pub fn average_triangles_per_block(scene: &SceneData) -> f32 {
    if scene.block_count == 0 {
        return 0.0;
    }
    scene.triangle_count as f32 / scene.block_count as f32
}

/// Average baked AO across vertical LOD0 faces, used by the M0a look test.
pub fn average_vertical_face_ao(scene: &SceneData) -> f32 {
    let mut sum = 0.0f32;
    let mut count = 0usize;
    for chunk in &scene.chunks {
        for (index, normal) in chunk.lod0.normals.iter().enumerate() {
            if normal[1].abs() <= 0.15 {
                sum += chunk.lod0.colors[index][0];
                count += 1;
            }
        }
    }
    if count == 0 {
        0.0
    } else {
        sum / count as f32
    }
}

/// Truncating helper kept public to mirror the quantised layout boundary.
pub fn quantise_mm(metres: f32) -> i64 {
    trunc(metres as f64 * 1000.0) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Gatehouse;
    use crate::fixtures::m0a_showcase;

    #[test]
    fn only_roof_blocks_carry_a_negative_encoded_height() {
        let terrain = SiteTerrain::for_site("lake_island").expect("lake_island is a curated site");
        let document = crate::fixtures::m2_baltic_document();
        let blocks = generate_blocks(&document, &terrain);
        let occupancy = Occupancy::build(&blocks, &terrain);

        let mut data = ChunkMeshData::default();
        let mut roof_vertices = 0usize;
        let mut stone_vertices = 0usize;
        for block in &blocks {
            let start = data.custom0.len();
            emit_block(&mut data, block, 0.5, &terrain, &occupancy);
            for custom in &data.custom0[start..] {
                if block.role == BlockRole::Roof {
                    assert!(
                        custom[3] < 0.0,
                        "roof block {} course {} emitted height {}",
                        block.element_id,
                        block.course_index,
                        custom[3]
                    );
                    roof_vertices += 1;
                } else {
                    assert!(
                        custom[3] > 0.0,
                        "non-roof block {:?} {} course {} emitted height {}",
                        block.role,
                        block.element_id,
                        block.course_index,
                        custom[3]
                    );
                    stone_vertices += 1;
                }
            }
        }
        assert!(roof_vertices > 0, "the showcase has no roof vertices");
        assert!(stone_vertices > 0, "the showcase has no stone vertices");
    }

    #[test]
    fn gatehouse_lod1_leaves_the_passage_open() {
        let terrain = SiteTerrain::m0a_promontory();
        let mut document = CastleDocument::new();
        let gatehouse = Gatehouse::new(1, "Gate", [0, 0], 0);
        document.add_gatehouse(gatehouse.clone());
        let blocks = generate_blocks(&document, &terrain);
        let refs: Vec<&Block> = blocks.iter().collect();
        let occupancy = Occupancy::build(&blocks, &terrain);
        let params = EmitParams {
            tint: 0.9,
            age: 0.8,
            jitter_seed: 0.0,
            build_norm: 0.5,
            course: 4.0,
            width: 1.0,
            height: 1.0,
        };
        let mut lod1 = ChunkMeshData::default();
        emit_lod1_gatehouse(&mut lod1, &gatehouse, &refs, &params, &terrain, &occupancy);
        assert!(!lod1.is_empty(), "gatehouse LOD1 produced no geometry");

        let ground_m = terrain.height_mm(0, 0) as f32 / 1000.0;
        let passage_top_m = ground_m + gatehouse.passage_height_mm as f32 / 1000.0;
        for position in &lod1.positions {
            let inside_passage = position[0].abs() < 3.5
                && position[2].abs() < 1.5
                && position[1] < passage_top_m - 0.05;
            assert!(
                !inside_passage,
                "gatehouse LOD1 blocks the passage at {position:?}"
            );
        }
        // Masonry still bridges above the gate.
        let above = lod1
            .positions
            .iter()
            .any(|position| position[1] >= passage_top_m);
        assert!(above, "gatehouse LOD1 has no masonry above the passage");
    }

    #[test]
    fn exposed_face_triangle_ratio_is_in_target_band() {
        let scene = m0a_showcase();
        let average = average_triangles_per_block(&scene);
        assert!(
            (6.0..=10.0).contains(&average),
            "average {average} tris/block outside 6..=10"
        );
        assert!(
            scene.triangle_count < scene.block_count * 12,
            "{} triangles is not below full 12-triangle cubes ({})",
            scene.triangle_count,
            scene.block_count * 12
        );
    }

    #[test]
    fn ao_is_in_unit_range() {
        let scene = m0a_showcase();
        for chunk in &scene.chunks {
            for color in &chunk.lod0.colors {
                assert!(
                    (0.0..=1.0).contains(&color[0]),
                    "ao {} out of range",
                    color[0]
                );
            }
        }
    }

    #[test]
    fn outward_vertical_faces_are_brightly_baked() {
        let scene = m0a_showcase();
        let average = average_vertical_face_ao(&scene);
        assert!(
            average > 0.50,
            "average outward vertical-face AO {average} is not > 0.50"
        );
    }
}
