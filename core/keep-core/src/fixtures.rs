//! Deterministic fixtures used by tests, benchmarks and the Godot bridge.
//!
//! * [`m0a_showcase`] — one battered curtain wall and one round tower on the
//!   fixed promontory: the M0a look/performance scene.
//! * [`m0b_showcase`] — two curtain walls fused to a battered drum tower, an
//!   angled wall-to-wall junction and two splayed arrow loops through a 2.5 m
//!   wall: the M0b junction and void scene.
//! * [`stress_200k`] — a large concentric castle used to test the 200k-block
//!   kill criterion. It produces just over 200k blocks, with realistic 24–28 m
//!   curtain walls and 24–28 m drum towers.

use crate::document::{
    BatteredWall, CastleDocument, Ditch, Gatehouse, Opening, RoundTower, TowerPlan,
};
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

/// The document behind the M0b junction/void look scene.
///
/// Curtain wall 1 is a 26 m, 2.5 m thick battered wall running into a battered
/// drum tower at its east end. Curtain wall 2 returns from the north at roughly
/// 115°, starting out on the sloping shoulder of the promontory (r > 25 m) so
/// the junction genuinely bridges two different ground elevations, and its
/// lower priority lets the junction solver trim it cleanly against wall 1. Two
/// splayed arrow loops pierce wall 1 at 28% and 47% along its run, clear of the
/// tower seam. Arrow loops always open toward `-d` (away from the bailey).
pub fn m0b_showcase_document() -> CastleDocument {
    let mut doc = CastleDocument::new();

    let mut wall = BatteredWall::new(1, "North curtain", [-8_000, 0], [18_000, 0], 9_000, 2_500);
    wall.batter_mm = 800;
    wall.plinth_height_mm = 900;
    wall.plinth_projection_mm = 300;
    wall.course_height_mm = 450;
    doc.add_wall(wall);

    // Lower-priority return wall, meeting wall 1 at its west end and rising
    // onto the sloping shoulder to the north (r > 25 m at its far end). Its
    // direction is about 115° from wall 1, so the junction is a real angled
    // seam between two battered walls at different ground elevations.
    let mut return_wall = BatteredWall::new(
        2,
        "North return",
        [4_000, 28_000],
        [-8_000, 0],
        8_000,
        2_200,
    );
    return_wall.batter_mm = 700;
    return_wall.plinth_height_mm = 800;
    return_wall.plinth_projection_mm = 260;
    return_wall.course_height_mm = 420;
    doc.add_wall(return_wall);

    // Battered drum tower at the east end of wall 1. The wall run penetrates
    // the tower footprint, so the junction solver trims it flush to the batter.
    let mut tower = RoundTower::new(3, "Drum tower", [19_000, 0], 4_200, 15_000);
    tower.thickness_mm = 1_300;
    tower.batter_mm = 600;
    tower.plinth_height_mm = 1_100;
    tower.plinth_projection_mm = 350;
    tower.course_height_mm = 450;
    doc.add_tower(tower);

    // Two splayed arrow loops through the 2.5 m wall.
    for (id, name, t) in [
        (10u64, "Arrow loop west", 280i64),
        (11, "Arrow loop east", 470),
    ] {
        let mut opening = Opening::arrow_loop(id, 1, name, t, 1_800);
        opening.splay_depth_mm = 2_500;
        doc.add_opening(opening);
    }

    doc
}

/// Build the M0b junction and void look scene.
pub fn m0b_showcase() -> SceneData {
    let terrain = SiteTerrain::m0a_promontory();
    let document = m0b_showcase_document();
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
        (88_000i64, 28_500i64, 3_000i64),
        (64_000, 24_500, 2_600),
        (36_000, 26_500, 3_200),
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

/// The document behind the M1 Edwardian showcase on the coastal headland.
///
/// It exercises every new M1 element: a twin-tower gatehouse, battered curtain
/// walls with arrow loops, one round, one square and one D-shaped tower, and a
/// defensive ditch across the landward approach.
pub fn m1_edwardian_document() -> CastleDocument {
    let mut doc = CastleDocument::new();

    let mut gatehouse = Gatehouse::new(1, "Great gatehouse", [0, 0], 0);
    gatehouse.passage_width_mm = 3_200;
    gatehouse.passage_height_mm = 4_200;
    gatehouse.depth_mm = 8_000;
    gatehouse.height_mm = 14_000;
    gatehouse.tower_radius_mm = 3_400;
    doc.add_gatehouse(gatehouse);

    // Landward curtains run west from the two flanking gatehouse towers.
    for (id, name, z) in [
        (2u64, "North curtain", -5_100i64),
        (3, "South curtain", 5_100),
    ] {
        let mut wall = BatteredWall::new(id, name, [-32_000, z], [0, z], 9_000, 2_400);
        wall.batter_mm = 900;
        wall.plinth_height_mm = 900;
        wall.plinth_projection_mm = 300;
        wall.course_height_mm = 450;
        doc.add_wall(wall);
    }

    let mut round = RoundTower::new(4, "West drum", [-32_000, -5_100], 3_600, 13_000);
    round.thickness_mm = 1_300;
    round.batter_mm = 600;
    round.plinth_height_mm = 1_000;
    doc.add_tower(round);

    let mut square = RoundTower::new(5, "Square tower", [-32_000, 5_100], 3_400, 13_000);
    square.thickness_mm = 1_300;
    square.batter_mm = 600;
    square.plinth_height_mm = 1_000;
    square.plan = TowerPlan::Square;
    square.orientation_deg = 0;
    doc.add_tower(square);

    let mut dshaped = RoundTower::new(6, "D-shaped tower", [-16_000, 16_000], 3_600, 13_000);
    dshaped.thickness_mm = 1_300;
    dshaped.batter_mm = 600;
    dshaped.plinth_height_mm = 1_000;
    dshaped.plan = TowerPlan::DShaped;
    dshaped.orientation_deg = 180;
    doc.add_tower(dshaped);

    // A defensive ditch across the landward approach.
    doc.add_ditch(Ditch::new(
        7,
        "Approach ditch",
        [-46_000, -20_000],
        [-46_000, 20_000],
        6_000,
        2_600,
    ));

    for (id, name, wall_id, t) in [
        (10u64, "North loop west", 2u64, 300i64),
        (11, "North loop east", 2, 620),
        (12, "South loop", 3, 480),
    ] {
        let mut opening = Opening::arrow_loop(id, wall_id, name, t, 1_800);
        opening.splay_depth_mm = 2_400;
        doc.add_opening(opening);
    }

    doc
}

/// Build the M1 Edwardian showcase scene on the coastal headland.
pub fn m1_edwardian_showcase() -> SceneData {
    let terrain =
        SiteTerrain::for_site("coastal_headland").expect("coastal_headland is a curated site");
    let document = m1_edwardian_document();
    build_scene(&document, &terrain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::StylePack;
    use crate::validate::validate_structure;

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
            (180_000..=230_000).contains(&scene.block_count),
            "stress fixture produced {} blocks, outside 180k..=230k",
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

    #[test]
    fn m0b_showcase_is_deterministic_and_has_openings() {
        let document = m0b_showcase_document();
        assert_eq!(document.opening_count(), 2);
        assert_eq!(document.len(), 3);

        let a = m0b_showcase();
        let b = m0b_showcase();
        assert_eq!(a.layout_hash, b.layout_hash);
        assert_eq!(a.block_count, b.block_count);
        assert_eq!(a.triangle_count, b.triangle_count);
        assert!(
            a.block_count > 1_500,
            "suspiciously few blocks: {}",
            a.block_count
        );
        assert!(!a.chunks.is_empty());
        for chunk in &a.chunks {
            for lod in [&chunk.lod0, &chunk.lod1, &chunk.lod2] {
                assert!(!lod.is_empty());
                assert_eq!(lod.positions.len(), lod.normals.len());
                assert_eq!(lod.indices.len() % 3, 0);
            }
        }
    }

    #[test]
    fn m1_edwardian_showcase_validates_and_builds() {
        let document = m1_edwardian_document();
        let terrain = SiteTerrain::for_site("coastal_headland").unwrap();
        let style = StylePack::edwardian_welsh();
        let report = validate_structure(&document, &terrain, &style);
        assert!(
            report.passed,
            "M1 showcase must pass: {:?}",
            report.findings
        );
        assert_eq!(report.score, 100, "findings: {:?}", report.findings);

        assert_eq!(document.gatehouses().count(), 1);
        assert_eq!(document.ditches().count(), 1);
        assert_eq!(document.opening_count(), 3);

        let scene = m1_edwardian_showcase();
        assert!(
            scene.block_count > 3_000,
            "scene had {} blocks",
            scene.block_count
        );
        assert!(!scene.chunks.is_empty());
        assert!(!scene.terrain_mesh.positions.is_empty());

        let other = m1_edwardian_showcase();
        assert_eq!(scene.layout_hash, other.layout_hash);
        assert_eq!(scene.block_count, other.block_count);
    }
}
