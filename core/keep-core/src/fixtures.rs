//! Deterministic fixtures used by tests, benchmarks and the Godot bridge.
//!
//! * [`m0a_showcase`] — one battered curtain wall and one round tower on the
//!   fixed promontory: the M0a look/performance scene.
//! * [`stress_200k`] — a large concentric castle used to test the 200k-block
//!   kill criterion. It produces just over 200k blocks, with realistic 24–28 m
//!   curtain walls and 24–28 m drum towers.

use crate::document::{BatteredWall, CastleDocument, RoundTower};
use crate::mesh::{build_scene, SceneData};
use crate::terrain::SiteTerrain;

/// The document behind the M0a look scene.
pub fn m0a_showcase_document() -> CastleDocument {
    let mut doc = CastleDocument::new();

    let mut wall = BatteredWall::new(1, "North curtain", [-15_000, 0], [12_400, 0], 9_000, 2_400);
    wall.batter_mm = 900;
    wall.plinth_height_mm = 900;
    wall.plinth_projection_mm = 300;
    wall.course_height_mm = 450;
    doc.add_wall(wall);

    let mut tower = RoundTower::new(2, "Drum tower", [16_000, 0], 3_500, 13_000);
    tower.thickness_mm = 1_200;
    tower.batter_mm = 500;
    tower.plinth_height_mm = 1_200;
    tower.plinth_projection_mm = 350;
    tower.course_height_mm = 450;
    doc.add_tower(tower);

    doc
}

/// Build the M0a look scene.
pub fn m0a_showcase() -> SceneData {
    let terrain = SiteTerrain::m0a_promontory();
    let document = m0a_showcase_document();
    build_scene(&document, &terrain)
}

/// The document behind the 200k-block stress scene.
pub fn stress_200k_document() -> CastleDocument {
    let mut doc = CastleDocument::new();
    let mut next_id = 100u64;

    // Three concentric square rings of curtain wall. The square plan is
    // intentionally simple: M0a is testing block count and meshing cost, while
    // M0b takes on junctions and angled runs.
    let rings = [
        (88_000i64, 28_000i64, 3_000i64),
        (64_000, 24_000, 2_600),
        (36_000, 26_000, 3_200),
    ];

    for (ring_index, (half, height, thickness)) in rings.into_iter().enumerate() {
        let height = height + ring_index as i64 * 100;
        let mut add_wall = |a: [i64; 2], b: [i64; 2], name: &str| {
            let mut wall = BatteredWall::new(next_id, name, a, b, height, thickness);
            next_id += 1;
            wall.batter_mm = 700;
            wall.plinth_height_mm = 700;
            wall.plinth_projection_mm = 250;
            wall.course_height_mm = 320;
            doc.add_wall(wall);
        };
        add_wall([-half, -half], [half, -half], "South curtain");
        add_wall([half, -half], [half, half], "East curtain");
        add_wall([half, half], [-half, half], "North curtain");
        add_wall([-half, half], [-half, -half], "West curtain");
    }

    // Corner and mid-wall drum towers on each ring.
    let tower_specs = [
        (88_000i64, 5_500i64, 28_000i64),
        (64_000, 4_500, 26_000),
        (36_000, 4_500, 24_000),
    ];
    for (half, radius, height) in tower_specs {
        let positions = [
            [-half, -half],
            [0, -half],
            [half, -half],
            [half, 0],
            [half, half],
            [0, half],
            [-half, half],
            [-half, 0],
        ];
        for (index, centre) in positions.into_iter().enumerate() {
            let mut tower =
                RoundTower::new(next_id, format!("Tower {index}"), centre, radius, height);
            next_id += 1;
            tower.batter_mm = 600;
            tower.plinth_height_mm = 900;
            tower.plinth_projection_mm = 300;
            tower.course_height_mm = 320;
            doc.add_tower(tower);
        }
    }

    doc
}

/// Build the 200k-block stress scene.
pub fn stress_200k() -> SceneData {
    let terrain = SiteTerrain::m0a_promontory();
    let document = stress_200k_document();
    build_scene(&document, &terrain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn showcase_is_deterministic() {
        let a = m0a_showcase();
        let b = m0a_showcase();
        assert_eq!(a.layout_hash, b.layout_hash);
        assert_eq!(a.block_count, b.block_count);
        assert_eq!(a.triangle_count, b.triangle_count);
        assert_eq!(a.chunks.len(), b.chunks.len());
        for (left, right) in a.chunks.iter().zip(&b.chunks) {
            assert_eq!(left.key, right.key);
            assert_eq!(left.centre, right.centre);
            assert_eq!(left.block_count, right.block_count);
            assert_eq!(left.lod0.positions, right.lod0.positions);
            assert_eq!(left.lod1.positions, right.lod1.positions);
            assert_eq!(left.lod2.positions, right.lod2.positions);
        }
    }

    #[test]
    fn showcase_has_non_empty_valid_chunks() {
        let scene = m0a_showcase();
        assert!(scene.block_count > 0);
        assert!(!scene.chunks.is_empty());
        for chunk in &scene.chunks {
            for lod in [&chunk.lod0, &chunk.lod1, &chunk.lod2] {
                assert!(!lod.is_empty(), "empty LOD in chunk {:?}", chunk.key);
                assert_eq!(lod.positions.len(), lod.normals.len());
                assert_eq!(lod.positions.len(), lod.uvs.len());
                assert_eq!(lod.positions.len(), lod.colors.len());
                assert_eq!(lod.positions.len(), lod.custom0.len());
                assert_eq!(lod.indices.len() % 3, 0);
                for p in &lod.positions {
                    assert!(p[0].is_finite() && p[1].is_finite() && p[2].is_finite());
                }
                for &index in &lod.indices {
                    assert!(index >= 0 && (index as usize) < lod.positions.len());
                }
            }
        }
    }

    #[test]
    fn stress_generates_about_200k_blocks() {
        let scene = stress_200k();
        assert!(
            (190_000..=230_000).contains(&scene.block_count),
            "stress fixture produced {} blocks, outside 190k..=230k",
            scene.block_count
        );
        assert!(!scene.chunks.is_empty());
        for chunk in &scene.chunks {
            assert!(!chunk.lod0.is_empty());
            assert!(!chunk.lod1.is_empty());
            assert!(!chunk.lod2.is_empty());
            for &index in &chunk.lod0.indices {
                assert!(index >= 0 && (index as usize) < chunk.lod0.positions.len());
            }
        }
    }
}
