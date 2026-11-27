//! Engine-agnostic picking and snapping.
//!
//! The live game, headless tests and offline agents all need the same answers
//! for "what ground is under this cursor" and "what should this endpoint snap
//! to". Keeping both in `keep-core` means picking never depends on the GPU or
//! on a loaded Godot scene.

use crate::document::{CastleDocument, Element, ElementId};
use crate::terrain::SiteTerrain;
use libm::{cos, round, sin, sqrt};

/// What a snapped point latched onto.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapKind {
    /// The centre of a round tower (highest priority snap).
    TowerCentre,
    /// The closest point on a round tower's outer circle.
    TowerPerimeter,
    /// The start or end of a curtain wall.
    WallEndpoint,
}

/// Result of a snapping query. `kind` is `None` when nothing was in range and
/// `point_mm` is then the unmodified input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapResult {
    pub point_mm: [i64; 2],
    pub kind: Option<SnapKind>,
    pub element_id: Option<ElementId>,
    /// Distance from the query point to the snap target, in millimetres.
    pub distance_mm: i64,
}

impl SnapResult {
    pub fn is_snapped(&self) -> bool {
        self.kind.is_some()
    }
}

/// Ray-march the terrain heightfield and refine the hit with a binary search.
///
/// `ray_origin_m` and `ray_dir_m` are in metres; the direction does not need to
/// be normalised. Returns the hit as `[x_mm, y_mm, z_mm]`, with `y_mm` snapped
/// to the sampled ground so the result sits exactly on the terrain authority.
pub fn pick_terrain(
    terrain: &SiteTerrain,
    ray_origin_m: [f32; 3],
    ray_dir_m: [f32; 3],
) -> Option<[i64; 3]> {
    let dx = ray_dir_m[0] as f64;
    let dy = ray_dir_m[1] as f64;
    let dz = ray_dir_m[2] as f64;
    let dir_len = sqrt(dx * dx + dy * dy + dz * dz);
    if dir_len < 1.0e-6 {
        return None;
    }
    let dir = [
        ray_dir_m[0] as f64 / dir_len,
        ray_dir_m[1] as f64 / dir_len,
        ray_dir_m[2] as f64 / dir_len,
    ];
    let origin = [
        ray_origin_m[0] as f64,
        ray_origin_m[1] as f64,
        ray_origin_m[2] as f64,
    ];

    let ground_at = |p: [f64; 3]| -> f64 { terrain.height_m(p[0] as f32, p[2] as f32) as f64 };
    let point_at = |t: f64| -> [f64; 3] {
        [
            origin[0] + dir[0] * t,
            origin[1] + dir[1] * t,
            origin[2] + dir[2] * t,
        ]
    };

    let mut prev_t = 0.0f64;
    let mut prev = origin;
    let mut prev_diff = prev[1] - ground_at(prev);
    if prev_diff <= 0.0 {
        // Already at or below the surface. If heading down, the origin is the
        // hit; if heading up, there is nothing ahead to hit.
        if dir[1] > 0.0 {
            return None;
        }
        return Some(hit_point(terrain, prev));
    }

    const STEP_M: f64 = 0.5;
    const MAX_T: f64 = 600.0;
    let mut t = STEP_M;
    while t <= MAX_T {
        let p = point_at(t);
        let diff = p[1] - ground_at(p);
        if diff <= 0.0 {
            // Binary-refine the crossing between the two samples.
            let mut lo = prev_t;
            let mut hi = t;
            for _ in 0..24 {
                let mid = (lo + hi) * 0.5;
                let pm = point_at(mid);
                if pm[1] - ground_at(pm) > 0.0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let hit = point_at((lo + hi) * 0.5);
            return Some(hit_point(terrain, hit));
        }
        prev_t = t;
        prev = p;
        prev_diff = diff;
        t += STEP_M;
        // Early exit if the ray has left the terrain well below the grid.
        // Both sides are in metres here; `extent_mm` must be converted.
        let extent_m = terrain.extent_mm().abs() as f64 / 1000.0;
        if prev[1] < -extent_m - 50.0 {
            break;
        }
    }
    let _ = prev_diff;
    None
}

fn hit_point(terrain: &SiteTerrain, p: [f64; 3]) -> [i64; 3] {
    let x_mm = round(p[0] * 1000.0) as i64;
    let z_mm = round(p[2] * 1000.0) as i64;
    let y_mm = terrain.height_mm(x_mm, z_mm);
    [x_mm, y_mm, z_mm]
}

/// Snap a plan point to nearby tower centres/perimeters or wall endpoints.
pub fn snap_point(
    document: &CastleDocument,
    point_mm: [i64; 2],
    snap_radius_mm: i64,
) -> SnapResult {
    let radius = snap_radius_mm.max(0);
    let radius_sq = (radius as i128) * (radius as i128);

    // A wall endpoint dropped anywhere on or near a tower should latch onto
    // the tower centre, not the endpoint: the junction solver can then trim
    // the wall cleanly into the tower with zero gap. This outranks every other
    // snap target while the point is inside `radius + snap_radius`.
    let mut centre: Option<(i128, u64, [i64; 2])> = None;
    for element in document.elements() {
        if let Element::RoundTower(tower) = element {
            let dx = tower.centre_mm[0] as i128 - point_mm[0] as i128;
            let dz = tower.centre_mm[1] as i128 - point_mm[1] as i128;
            let dist_sq = dx * dx + dz * dz;
            let limit = tower.radius_mm as i128 + radius as i128;
            if dist_sq > limit * limit {
                continue;
            }
            let replace = match centre {
                None => true,
                Some((best_dist, best_id, _)) => (dist_sq, tower.id) < (best_dist, best_id),
            };
            if replace {
                centre = Some((dist_sq, tower.id, tower.centre_mm));
            }
        }
    }
    if let Some((dist_sq, id, candidate)) = centre {
        return SnapResult {
            point_mm: candidate,
            kind: Some(SnapKind::TowerCentre),
            element_id: Some(id),
            distance_mm: sqrt(dist_sq as f64) as i64,
        };
    }

    let mut best: Option<(i128, SnapKind, u64, [i64; 2])> = None;
    let mut consider = |candidate_mm: [i64; 2], kind: SnapKind, id: u64| {
        let dx = candidate_mm[0] as i128 - point_mm[0] as i128;
        let dz = candidate_mm[1] as i128 - point_mm[1] as i128;
        let dist_sq = dx * dx + dz * dz;
        if dist_sq > radius_sq {
            return;
        }
        let key = (dist_sq, kind, id);
        let replace = match &best {
            None => true,
            Some((best_dist, best_kind, best_id, _)) => {
                (dist_sq, kind, id) < (*best_dist, *best_kind, *best_id)
            }
        };
        if replace {
            best = Some((key.0, key.1, key.2, candidate_mm));
        }
    };

    for element in document.elements() {
        match element {
            Element::RoundTower(tower) => {
                consider(tower.centre_mm, SnapKind::TowerCentre, tower.id);
                let dx = point_mm[0] as f64 - tower.centre_mm[0] as f64;
                let dz = point_mm[1] as f64 - tower.centre_mm[1] as f64;
                let len = sqrt(dx * dx + dz * dz);
                if len > 1.0e-6 {
                    let perimeter = [
                        round(tower.centre_mm[0] as f64 + dx / len * tower.radius_mm as f64) as i64,
                        round(tower.centre_mm[1] as f64 + dz / len * tower.radius_mm as f64) as i64,
                    ];
                    consider(perimeter, SnapKind::TowerPerimeter, tower.id);
                }
            }
            Element::Gatehouse(gatehouse) => {
                consider(gatehouse.centre_mm, SnapKind::TowerCentre, gatehouse.id);
                for tower in gatehouse.flank_towers() {
                    consider(tower.centre_mm, SnapKind::TowerCentre, gatehouse.id);
                }
            }
            Element::Keep(keep) => {
                consider(keep.centre_mm, SnapKind::TowerCentre, keep.id);
                let angle = keep.orientation_deg as f64 * std::f64::consts::PI / 180.0;
                let (c, s) = (cos(angle), sin(angle));
                let half_w = keep.width_mm.max(1) as f64 * 0.5;
                let half_d = keep.depth_mm.max(1) as f64 * 0.5;
                for (u, v) in [
                    (-half_w, -half_d),
                    (half_w, -half_d),
                    (half_w, half_d),
                    (-half_w, half_d),
                ] {
                    let x = keep.centre_mm[0] as f64 + u * c - v * s;
                    let z = keep.centre_mm[1] as f64 + u * s + v * c;
                    consider(
                        [round(x) as i64, round(z) as i64],
                        SnapKind::WallEndpoint,
                        keep.id,
                    );
                }
            }
            Element::Range(range) => {
                consider(range.centre_mm, SnapKind::TowerCentre, range.id);
                let angle = range.orientation_deg as f64 * std::f64::consts::PI / 180.0;
                let (c, s) = (cos(angle), sin(angle));
                let half = range.length_mm.max(1) as f64 * 0.5;
                for sign in [-1.0f64, 1.0] {
                    let x = range.centre_mm[0] as f64 + half * sign * c;
                    let z = range.centre_mm[1] as f64 + half * sign * s;
                    consider(
                        [round(x) as i64, round(z) as i64],
                        SnapKind::WallEndpoint,
                        range.id,
                    );
                }
            }
            Element::Ditch(ditch) => {
                consider(ditch.start_mm, SnapKind::WallEndpoint, ditch.id);
                consider(ditch.end_mm, SnapKind::WallEndpoint, ditch.id);
            }
            Element::BatteredWall(wall) => {
                consider(wall.start_mm, SnapKind::WallEndpoint, wall.id);
                consider(wall.end_mm, SnapKind::WallEndpoint, wall.id);
            }
            // A terrain sculpt is ground, not masonry. Snapping its centre as a
            // `TowerCentre` would yank walls to the middle of a hill, so it is
            // deliberately not a snap target (M3.1 H1).
            Element::TerrainSculpt(_) => {}
        }
    }

    match best {
        Some((dist_sq, kind, id, candidate)) => SnapResult {
            point_mm: candidate,
            kind: Some(kind),
            element_id: Some(id),
            distance_mm: sqrt(dist_sq as f64) as i64,
        },
        None => SnapResult {
            point_mm,
            kind: None,
            element_id: None,
            distance_mm: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{BatteredWall, RoundTower};

    #[test]
    fn terrain_pick_hits_the_plateau_from_above() {
        let terrain = SiteTerrain::m0a_promontory();
        let hit = pick_terrain(&terrain, [0.0, 60.0, 0.0], [0.0, -1.0, 0.0]).expect("a hit");
        assert_eq!(hit[0], 0);
        assert_eq!(hit[2], 0);
        let expected = terrain.height_mm(0, 0);
        assert!(
            (hit[1] - expected).abs() < 5,
            "picked {} vs terrain {expected}",
            hit[1]
        );
    }

    #[test]
    fn terrain_pick_returns_none_when_pointing_away() {
        let terrain = SiteTerrain::m0a_promontory();
        assert!(pick_terrain(&terrain, [0.0, 60.0, 0.0], [0.0, 1.0, 0.0]).is_none());
    }

    #[test]
    fn snapping_near_a_tower_prefers_its_centre_over_a_wall_endpoint() {
        let mut doc = CastleDocument::new();
        doc.add_wall(BatteredWall::new(
            1,
            "wall",
            [0, 0],
            [16_000, 0],
            9_000,
            2_000,
        ));
        doc.add_tower(RoundTower::new(2, "tower", [20_000, 0], 4_000, 12_000));

        // The point is only ~110 mm from the wall endpoint, but it lies inside
        // the tower's radius + snap radius, so the centre must win.
        let result = snap_point(&doc, [16_100, 50], 1_500);
        assert_eq!(result.kind, Some(SnapKind::TowerCentre));
        assert_eq!(result.point_mm, [20_000, 0]);
    }

    #[test]
    fn snap_prefers_tower_centre_then_perimeter_then_wall_endpoint() {
        let mut doc = CastleDocument::new();
        doc.add_wall(BatteredWall::new(
            1,
            "wall",
            [0, 0],
            [10_000, 0],
            9_000,
            2_000,
        ));
        doc.add_tower(RoundTower::new(2, "tower", [20_000, 0], 4_000, 12_000));

        let centre = snap_point(&doc, [19_800, 300], 2_000);
        assert_eq!(centre.kind, Some(SnapKind::TowerCentre));
        assert_eq!(centre.point_mm, [20_000, 0]);

        let endpoint = snap_point(&doc, [10_200, 400], 2_000);
        assert_eq!(endpoint.kind, Some(SnapKind::WallEndpoint));
        assert_eq!(endpoint.point_mm, [10_000, 0]);

        let far = snap_point(&doc, [5_000, 5_000], 500);
        assert!(!far.is_snapped());
        assert_eq!(far.point_mm, [5_000, 5_000]);
    }
}
