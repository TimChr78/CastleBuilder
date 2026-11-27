//! Priority-ordered junction solving.
//!
//! Real junctions are volumetric: a battered curtain wall runs into a battered
//! round tower, or two curtain walls meet at an angle. Instead of letting the
//! two elements' stones interpenetrate, the solver trims the lower-priority
//! element's *run intervals* per course so its stones stop flush at the
//! higher-priority element's surface. The masonry layout then only emits stones
//! inside those allowed intervals and dresses the trimmed ends as seam quoins.
//!
//! Priority is `RoundTower > BatteredWall`, with ties broken by the lower
//! [`ElementId`] winning. Everything is integer millimetres and deterministic;
//! the only floating-point work is the analytic circle/line intersection, which
//! uses `libm`.
//!
//! Two details keep the seams watertight:
//!
//! * The higher-priority wall of an angled wall-to-wall corner is extended
//!   slightly past the shared endpoint so the outer mitre is solid; the lower
//!   wall is trimmed to the incumbent's *course-elevation* face with a small
//!   tuck-in so no daylight opens between the two runs.
//! * Wall-to-tower trims are solved independently at the wall's two faces and
//!   at the course bottom/top, producing a trapezoidal seam quoin. The trim is
//!   pushed a little into the tower shell so the flat tower facets never show a
//!   gap against the true circle.

use std::collections::BTreeSet;

use crate::document::{BatteredWall, CastleDocument, Element, ElementId, RoundTower};
use crate::terrain::SiteTerrain;
use libm::{round, sqrt};

/// Narrow tolerance used when a seam is allowed to sit a hair outside the
/// higher-priority surface so quantisation never pushes a stone inside it.
pub const SEAM_TOLERANCE_MM: i64 = 2;
/// How far the lower-priority wall is tucked behind the incumbent's face.
pub const SEAM_TUCK_MM: i64 = 20;
/// How far a wall is pushed into a tower shell to hide its faceted surface.
pub const TOWER_BURY_MM: i64 = 40;

/// Relative priority of an element kind. Higher wins.
fn kind_priority(element: &Element) -> i32 {
    match element {
        Element::RoundTower(_) => 3,
        Element::Gatehouse(_) => 2,
        Element::BatteredWall(_) => 1,
        Element::Ditch(_) => 0,
    }
}

/// True when `incumbent` must be trimmed by `candidate`.
fn outranks(incumbent: &Element, candidate: &Element) -> bool {
    element_outranks(incumbent, kind_priority(candidate), candidate.id())
}

fn element_outranks(incumbent: &Element, candidate_priority: i32, candidate_id: ElementId) -> bool {
    let priority = kind_priority(incumbent);
    priority > candidate_priority
        || (priority == candidate_priority && incumbent.id() < candidate_id)
}

/// True when a plain [`BatteredWall`] outranks `candidate`.
fn wall_outranks(wall: &BatteredWall, candidate: &Element) -> bool {
    let priority = 1;
    let candidate_priority = kind_priority(candidate);
    priority > candidate_priority || (priority == candidate_priority && wall.id < candidate.id())
}

/// Ground height range sampled along a wall run. Kept here so the junction
/// solver and the masonry layout agree on the wall's silhouette exactly.
pub fn wall_ground_range(wall: &BatteredWall, terrain: &SiteTerrain) -> (i64, i64) {
    let dx = (wall.end_mm[0] - wall.start_mm[0]) as f64;
    let dz = (wall.end_mm[1] - wall.start_mm[1]) as f64;
    let len = sqrt(dx * dx + dz * dz);
    if len < 1.0 {
        let h = terrain.height_mm(wall.start_mm[0], wall.start_mm[1]);
        return (h, h);
    }
    let dir = [dx / len, dz / len];
    const SAMPLES: i32 = 24;
    let mut min_h = i64::MAX;
    let mut max_h = i64::MIN;
    for i in 0..=SAMPLES {
        let t = i as f64 / SAMPLES as f64;
        let x = wall.start_mm[0] as f64 + dir[0] * len * t;
        let z = wall.start_mm[1] as f64 + dir[1] * len * t;
        let h = terrain.height_mm(round(x) as i64, round(z) as i64);
        min_h = min_h.min(h);
        max_h = max_h.max(h);
    }
    (min_h, max_h)
}

/// Foundation, plinth-top and top elevation of a wall, matching the masonry
/// layout. The tuple is `(foundation_y, plinth_top_y, top_y)`.
pub fn wall_elevations(wall: &BatteredWall, terrain: &SiteTerrain) -> (i64, i64, i64) {
    let (ground_min, ground_max) = wall_ground_range(wall, terrain);
    (
        ground_min - 1_200,
        ground_max + wall.plinth_height_mm,
        ground_max + wall.height_mm,
    )
}

/// Actual half-width of a wall's cross-section at elevation `y_mm`, including
/// the projecting plinth while that elevation is in the plinth zone.
pub fn wall_half_width_at(wall: &BatteredWall, terrain: &SiteTerrain, y_mm: i64) -> i64 {
    let (foundation_y, plinth_top_y, top_y) = wall_elevations(wall, terrain);
    let total_height = (top_y - foundation_y).max(1);
    let t = ((y_mm - foundation_y) as f64 / total_height as f64).clamp(0.0, 1.0);
    let batter_inset = round(wall.batter_mm as f64 * t) as i64;
    let plinth = if y_mm < plinth_top_y {
        wall.plinth_projection_mm
    } else {
        0
    };
    (wall.thickness_mm / 2 - batter_inset / 2 + plinth).max(1)
}

/// Ground height range sampled around a tower's footprint. Kept here so the
/// junction solver and the masonry layout agree on the tower's battered
/// silhouette exactly.
pub fn tower_ground_range(tower: &RoundTower, terrain: &SiteTerrain) -> (i64, i64) {
    let mut min_h = terrain.height_mm(tower.centre_mm[0], tower.centre_mm[1]);
    let mut max_h = min_h;
    for i in 0..12 {
        let angle = std::f64::consts::TAU * i as f64 / 12.0;
        let x = tower.centre_mm[0] as f64 + libm::cos(angle) * tower.radius_mm as f64;
        let z = tower.centre_mm[1] as f64 + libm::sin(angle) * tower.radius_mm as f64;
        let h = terrain.height_mm(round(x) as i64, round(z) as i64);
        min_h = min_h.min(h);
        max_h = max_h.max(h);
    }
    (min_h, max_h)
}

/// Foundation and top elevation of a tower, matching the masonry layout.
pub fn tower_elevations(tower: &RoundTower, terrain: &SiteTerrain) -> (i64, i64, i64) {
    let (ground_min, ground_max) = tower_ground_range(tower, terrain);
    (
        ground_min - 1_200,
        ground_max + tower.plinth_height_mm,
        ground_max + tower.height_mm,
    )
}

/// Outer radius of the battered tower at elevation `y_mm`, including the
/// projecting plinth while it is in the plinth zone.
pub fn tower_outer_radius_at(tower: &RoundTower, terrain: &SiteTerrain, y_mm: i64) -> i64 {
    let (foundation_y, plinth_top_y, top_y) = tower_elevations(tower, terrain);
    let total_height = (top_y - foundation_y).max(1);
    let t = ((y_mm - foundation_y) as f64 / total_height as f64).clamp(0.0, 1.0);
    let batter_inset = round(tower.batter_mm as f64 * t) as i64;
    let plinth = if y_mm < plinth_top_y {
        tower.plinth_projection_mm
    } else {
        0
    };
    (tower.radius_mm - batter_inset / 2 + plinth).max(400)
}

/// Inner radius of the battered tower shell at elevation `y_mm`.
pub fn tower_inner_radius_at(tower: &RoundTower, terrain: &SiteTerrain, y_mm: i64) -> i64 {
    let (foundation_y, plinth_top_y, top_y) = tower_elevations(tower, terrain);
    let total_height = (top_y - foundation_y).max(1);
    let t = ((y_mm - foundation_y) as f64 / total_height as f64).clamp(0.0, 1.0);
    let batter_inset = round(tower.batter_mm as f64 * t) as i64;
    let plinth = if y_mm < plinth_top_y {
        tower.plinth_projection_mm
    } else {
        0
    };
    (tower.radius_mm - tower.thickness_mm - batter_inset / 2 - plinth).max(80)
}

fn wall_frame(wall: &BatteredWall) -> Option<([f64; 2], [f64; 2], f64)> {
    let dx = (wall.end_mm[0] - wall.start_mm[0]) as f64;
    let dz = (wall.end_mm[1] - wall.start_mm[1]) as f64;
    let len = sqrt(dx * dx + dz * dz);
    if len < 1.0 {
        return None;
    }
    let dir = [dx / len, dz / len];
    Some((dir, [-dir[1], dir[0]], len))
}

/// How far a wall must be extended past each end so an angled corner with a
/// lower-priority wall is completely closed with solid masonry.
///
/// Returns `(extend_start, extend_end)` in run millimetres; both are
/// non-negative and measured outward from the wall's own endpoints.
pub fn wall_corner_extensions(
    document: &CastleDocument,
    wall: &BatteredWall,
    y0: i64,
    y1: i64,
    terrain: &SiteTerrain,
) -> (i64, i64) {
    let Some((dir, _, _)) = wall_frame(wall) else {
        return (0, 0);
    };
    const ENDPOINT_TOLERANCE_MM: i64 = 60;
    let near = |a: [i64; 2], b: [i64; 2]| -> bool {
        let dx = a[0] as i128 - b[0] as i128;
        let dz = a[1] as i128 - b[1] as i128;
        dx * dx + dz * dz <= (ENDPOINT_TOLERANCE_MM as i128) * (ENDPOINT_TOLERANCE_MM as i128)
    };

    let mut extend_start = 0i64;
    let mut extend_end = 0i64;
    for element in document.elements() {
        let Element::BatteredWall(other) = element else {
            continue;
        };
        if other.id == wall.id || !wall_outranks(wall, element) {
            continue;
        }
        let Some((odir, _, _)) = wall_frame(other) else {
            continue;
        };
        // Near-parallel walls have no meaningful mitre; extending them would
        // create a long, spurious protrusion instead of closing a corner.
        let sin_theta = (dir[0] * odir[1] - dir[1] * odir[0]).abs();
        if sin_theta < 0.15 {
            continue;
        }
        let other_half = wall_half_width_at(other, terrain, y0)
            .max(wall_half_width_at(other, terrain, y1))
            .max(1) as f64;
        let cap = (other.thickness_mm.saturating_mul(3)).max(2_000) as f64;
        let ext = ((other_half / sin_theta).round() as i64).min(cap as i64);
        if near(other.start_mm, wall.start_mm) || near(other.end_mm, wall.start_mm) {
            extend_start = extend_start.max(ext);
        }
        if near(other.start_mm, wall.end_mm) || near(other.end_mm, wall.end_mm) {
            extend_end = extend_end.max(ext);
        }
    }
    (extend_start, extend_end)
}

/// Allowed run intervals for one wall course after trimming against every
/// higher-priority element. Intervals are in wall-local run millimetres,
/// clipped to `[run_start, run_end]`, sorted and non-overlapping.
#[allow(clippy::too_many_arguments)]
pub fn wall_course_intervals(
    document: &CastleDocument,
    wall: &BatteredWall,
    d0_bot: i64,
    d1_bot: i64,
    d0_top: i64,
    d1_top: i64,
    run_start: i64,
    run_end: i64,
    y0: i64,
    y1: i64,
    terrain: &SiteTerrain,
) -> Vec<(i64, i64)> {
    if run_end <= run_start {
        return Vec::new();
    }
    let Some((dir, right, _)) = wall_frame(wall) else {
        return Vec::new();
    };
    let mut allowed = vec![(run_start, run_end)];

    for element in document.elements() {
        if !element_outranks(element, 1, wall.id) {
            continue;
        }
        match element {
            Element::RoundTower(tower) => {
                if let Some((lo, hi)) = blocked_by_tower(
                    wall, dir, right, d0_bot, d1_bot, d0_top, d1_top, run_start, run_end, y0, y1,
                    tower, terrain,
                ) {
                    if hi > lo {
                        allowed = subtract_interval(&allowed, lo, hi);
                    }
                }
            }
            Element::Gatehouse(gatehouse) => {
                for tower in gatehouse.flank_towers() {
                    if let Some((lo, hi)) = blocked_by_tower(
                        wall, dir, right, d0_bot, d1_bot, d0_top, d1_top, run_start, run_end, y0,
                        y1, &tower, terrain,
                    ) {
                        if hi > lo {
                            allowed = subtract_interval(&allowed, lo, hi);
                        }
                    }
                }
            }
            Element::BatteredWall(other) => {
                if let Some((lo, hi)) = blocked_by_wall(
                    document, wall, dir, right, d0_bot, d1_bot, run_start, run_end, y0, y1, other,
                    terrain,
                ) {
                    if hi > lo {
                        allowed = subtract_interval(&allowed, lo, hi);
                    }
                }
            }
            Element::Ditch(_) => {}
        }
    }
    allowed
}

/// Per-face roots of the intersection between a wall face offset `d` from the
/// wall centreline and a tower circle at elevation `y`. Returns
/// `(near, far)` in wall-run millimetres, rounded to whole millimetres.
fn tower_face_roots(
    wall: &BatteredWall,
    dir: [f64; 2],
    right: [f64; 2],
    d: i64,
    y: i64,
    tower: &RoundTower,
    terrain: &SiteTerrain,
) -> Option<(i64, i64)> {
    let radius = (tower_outer_radius_at(tower, terrain, y) - TOWER_BURY_MM)
        .max(tower_inner_radius_at(tower, terrain, y) + 50);
    let centre = [tower.centre_mm[0] as f64, tower.centre_mm[1] as f64];
    let start = [wall.start_mm[0] as f64, wall.start_mm[1] as f64];
    let q = [
        start[0] + right[0] * d as f64,
        start[1] + right[1] * d as f64,
    ];
    let w = [q[0] - centre[0], q[1] - centre[1]];
    let b = w[0] * dir[0] + w[1] * dir[1];
    let c = w[0] * w[0] + w[1] * w[1] - (radius * radius) as f64;
    let disc = b * b - c;
    if disc <= 0.0 {
        return None;
    }
    let root = sqrt(disc);
    Some((round(-b - root) as i64, round(-b + root) as i64))
}

#[allow(clippy::too_many_arguments)]
fn blocked_by_tower(
    wall: &BatteredWall,
    dir: [f64; 2],
    right: [f64; 2],
    d0_bot: i64,
    d1_bot: i64,
    d0_top: i64,
    d1_top: i64,
    run_start: i64,
    run_end: i64,
    y0: i64,
    y1: i64,
    tower: &RoundTower,
    terrain: &SiteTerrain,
) -> Option<(i64, i64)> {
    let (_, _, tower_top) = tower_elevations(tower, terrain);
    if y0 >= tower_top {
        return None;
    }
    let mut near_max = i64::MIN;
    let mut far_min = i64::MAX;
    let mut any = false;
    for (d, y) in [(d0_bot, y0), (d1_bot, y0), (d0_top, y1), (d1_top, y1)] {
        if let Some((near, far)) = tower_face_roots(wall, dir, right, d, y, tower, terrain) {
            near_max = near_max.max(near);
            far_min = far_min.min(far);
            any = true;
        }
    }
    if !any {
        return None;
    }
    let lo = near_max.clamp(run_start, run_end);
    let hi = far_min.clamp(run_start, run_end);
    (hi > lo).then_some((lo, hi))
}

#[allow(clippy::too_many_arguments)]
fn blocked_by_wall(
    document: &CastleDocument,
    wall: &BatteredWall,
    dir: [f64; 2],
    right: [f64; 2],
    d0: i64,
    d1: i64,
    run_start: i64,
    run_end: i64,
    y0: i64,
    y1: i64,
    other: &BatteredWall,
    terrain: &SiteTerrain,
) -> Option<(i64, i64)> {
    let other_length = other.length_mm();
    if other_length <= 0 {
        return None;
    }
    let (odir, oright, olen) = wall_frame(other)?;
    if olen < 1.0 {
        return None;
    }
    let (oext_start, oext_end) = wall_corner_extensions(document, other, y0, y1, terrain);

    // Only trim when the two wall runs are near enough to interact. The
    // tolerance covers both wall thicknesses, the incumbent's corner
    // extension and a little slack for hand-dragged endpoints.
    let reach = (wall.thickness_mm + other.thickness_mm) as f64 * 0.5
        + oext_start.max(oext_end) as f64
        + 750.0;
    if segment_distance(
        [wall.start_mm[0] as f64, wall.start_mm[1] as f64],
        [wall.end_mm[0] as f64, wall.end_mm[1] as f64],
        [other.start_mm[0] as f64, other.start_mm[1] as f64],
        [other.end_mm[0] as f64, other.end_mm[1] as f64],
    ) > reach
    {
        return None;
    }

    // The incumbent's face at the top of this course. Using the top (most
    // battered) elevation guarantees the lower wall never stops outside the
    // incumbent's upper stones, and the tuck pushes it just behind the face.
    let half = (wall_half_width_at(other, terrain, y1).max(1) - SEAM_TUCK_MM).max(1) as f64;

    // Closest point of this wall's cross-section to the incumbent's face.
    let start = [wall.start_mm[0] as f64, wall.start_mm[1] as f64];
    let rel = [
        start[0] - other.start_mm[0] as f64,
        start[1] - other.start_mm[1] as f64,
    ];
    let base = rel[0] * oright[0] + rel[1] * oright[1];
    let slope = dir[0] * oright[0] + dir[1] * oright[1];
    if slope.abs() < 1.0e-6 {
        // Parallel walls: no meaningful mitre. Leave the layout untouched.
        return None;
    }
    // The lower wall is a slab, so its two faces sit at `d0` and `d1` across
    // the run. Solve the blocked interval over the whole cross-section range,
    // not just its nearest point, so no corner can slip inside the incumbent.
    let k = right[0] * oright[0] + right[1] * oright[1];
    let f0 = base + d0 as f64 * k;
    let f1 = base + d1 as f64 * k;
    let f_min = f0.min(f1);
    let f_max = f0.max(f1);
    let a1 = (half - f_min) / slope;
    let a2 = (-half - f_max) / slope;
    let mut lo = a1.min(a2);
    let mut hi = a1.max(a2);

    // Clip the blocked interval to the incumbent's *actual* extent along its
    // own run. The infinite-slab solve otherwise cuts holes in walls that
    // merely line up with the incumbent's plane but never touch it.
    let run_base = rel[0] * odir[0] + rel[1] * odir[1];
    let run_slope = dir[0] * odir[0] + dir[1] * odir[1];
    if run_slope.abs() < 1.0e-6 {
        return None;
    }
    let k_run = right[0] * odir[0] + right[1] * odir[1];
    let r0 = run_base + d0 as f64 * k_run;
    let r1 = run_base + d1 as f64 * k_run;
    let r_min = r0.min(r1);
    let r_max = r0.max(r1);
    let o_run_lo = -oext_start as f64;
    let o_run_hi = other_length as f64 + oext_end as f64;
    let e1 = (o_run_lo - r_max) / run_slope;
    let e2 = (o_run_hi - r_min) / run_slope;
    lo = lo.max(e1.min(e2)).max(run_start as f64);
    hi = hi.min(e1.max(e2)).min(run_end as f64);
    if hi <= lo {
        return None;
    }
    Some((lo.floor() as i64, hi.ceil() as i64))
}

/// A wall-to-tower seam at the forward (`run_end`) or backward (`run_start`)
/// end of a wall course. The four `a_*` values are the run positions of the
/// seam quoin's end corners `(d0,y0)`, `(d1,y0)`, `(d0,y1)`, `(d1,y1)`.
#[derive(Clone, Copy, Debug)]
pub struct TowerSeam {
    pub forward: bool,
    /// Run boundary the masonry interval was trimmed to.
    pub nominal: i64,
    pub corners: [i64; 4],
}

/// Resolve the trapezoidal seam profile for a wall course that terminates in a
/// round tower. `d*_bot`/`d*_top` are the face offsets at the course bed and
/// top; `y0`/`y1` its bottom and top elevations.
#[allow(clippy::too_many_arguments)]
pub fn tower_seam(
    document: &CastleDocument,
    wall: &BatteredWall,
    d0_bot: i64,
    d1_bot: i64,
    d0_top: i64,
    d1_top: i64,
    y0: i64,
    y1: i64,
    terrain: &SiteTerrain,
) -> Option<TowerSeam> {
    let (dir, right, _) = wall_frame(wall)?;
    let length = wall.length_mm();
    for element in document.elements() {
        if !element_outranks(element, 1, wall.id) {
            continue;
        }
        let Element::RoundTower(tower) = element else {
            continue;
        };
        let (_, _, tower_top) = tower_elevations(tower, terrain);
        if y0 >= tower_top {
            continue;
        }
        let mut roots: [Option<(i64, i64)>; 4] = [None; 4];
        for (index, (d, y)) in [(d0_bot, y0), (d1_bot, y0), (d0_top, y1), (d1_top, y1)]
            .into_iter()
            .enumerate()
        {
            roots[index] = tower_face_roots(wall, dir, right, d, y, tower, terrain);
        }
        let valid: Vec<(i64, i64)> = roots.iter().flatten().copied().collect();
        if valid.is_empty() {
            continue;
        }
        let near_max = valid.iter().map(|r| r.0).max().unwrap_or(0);
        let far_min = valid.iter().map(|r| r.1).min().unwrap_or(0);
        if near_max < length && far_min > length {
            let nominal = valid.iter().map(|r| r.0).max().unwrap_or(0);
            let mut corners = [nominal; 4];
            for (index, root) in roots.iter().enumerate() {
                if let Some(root) = root {
                    corners[index] = root.0;
                }
            }
            return Some(TowerSeam {
                forward: true,
                nominal,
                corners,
            });
        }
        if near_max < 0 && far_min > 0 {
            let nominal = valid.iter().map(|r| r.1).min().unwrap_or(0);
            let mut corners = [nominal; 4];
            for (index, root) in roots.iter().enumerate() {
                if let Some(root) = root {
                    corners[index] = root.1;
                }
            }
            return Some(TowerSeam {
                forward: false,
                nominal,
                corners,
            });
        }
    }
    None
}

/// A wall-to-wall seam at the forward (`end`) or backward (`start`) end of the
/// lower-priority wall. The four `a_*` values are the run positions where the
/// end corners `(d0,y0)`, `(d1,y0)`, `(d0,y1)`, `(d1,y1)` meet the incumbent's
/// face (tucked slightly behind it).
#[derive(Clone, Copy, Debug)]
pub struct WallSeam {
    pub forward: bool,
    pub corners: [i64; 4],
}

fn endpoints_touch(a: [i64; 2], b: [i64; 2]) -> bool {
    let dx = a[0] as i128 - b[0] as i128;
    let dz = a[1] as i128 - b[1] as i128;
    const TOL: i128 = 60;
    dx * dx + dz * dz <= TOL * TOL
}

/// Resolve the mitre profiles for a lower-priority wall whose ends meet
/// higher-priority walls. Returns at most one seam per end, choosing the
/// highest-priority incumbent at a shared endpoint.
#[allow(clippy::too_many_arguments)]
pub fn wall_seams(
    document: &CastleDocument,
    wall: &BatteredWall,
    d0_bot: i64,
    d1_bot: i64,
    d0_top: i64,
    d1_top: i64,
    y0: i64,
    y1: i64,
    terrain: &SiteTerrain,
) -> Vec<WallSeam> {
    let mut seams = Vec::new();
    let Some((dir, right, _)) = wall_frame(wall) else {
        return seams;
    };
    for forward in [true, false] {
        let mut best: Option<(ElementId, WallSeam)> = None;
        for element in document.elements() {
            if !element_outranks(element, 1, wall.id) {
                continue;
            }
            let Element::BatteredWall(other) = element else {
                continue;
            };
            if other.id == wall.id {
                continue;
            }
            let shares = if forward {
                endpoints_touch(wall.end_mm, other.start_mm)
                    || endpoints_touch(wall.end_mm, other.end_mm)
            } else {
                endpoints_touch(wall.start_mm, other.start_mm)
                    || endpoints_touch(wall.start_mm, other.end_mm)
            };
            if !shares {
                continue;
            }
            let Some((_, oright, _)) = wall_frame(other) else {
                continue;
            };
            let shared = if forward { wall.end_mm } else { wall.start_mm };
            let rel_shared = [
                shared[0] as f64 - other.start_mm[0] as f64,
                shared[1] as f64 - other.start_mm[1] as f64,
            ];
            let signed = rel_shared[0] * oright[0] + rel_shared[1] * oright[1];
            let sign = if signed >= 0.0 { 1.0 } else { -1.0 };

            let rel = [
                wall.start_mm[0] as f64 - other.start_mm[0] as f64,
                wall.start_mm[1] as f64 - other.start_mm[1] as f64,
            ];
            let base = rel[0] * oright[0] + rel[1] * oright[1];
            let slope = dir[0] * oright[0] + dir[1] * oright[1];
            if slope.abs() < 0.15 {
                // Nearly parallel walls share a long, ill-conditioned (and
                // meaningless) mitre; leave the flat run cut alone.
                continue;
            }
            let k = right[0] * oright[0] + right[1] * oright[1];
            let mut corners = [0i64; 4];
            for (index, (d, y)) in [(d0_bot, y0), (d1_bot, y0), (d0_top, y1), (d1_top, y1)]
                .into_iter()
                .enumerate()
            {
                let half = (wall_half_width_at(other, terrain, y).max(1) - SEAM_TUCK_MM) as f64;
                let target = sign * half;
                let a = (target - base - d as f64 * k) / slope;
                corners[index] = round(a) as i64;
            }
            let candidate = WallSeam { forward, corners };
            if best.as_ref().is_none_or(|(id, _)| other.id < *id) {
                best = Some((other.id, candidate));
            }
        }
        if let Some((_, seam)) = best {
            seams.push(seam);
        }
    }
    seams
}

/// Shortest distance between two 2D line segments.
fn segment_distance(a0: [f64; 2], a1: [f64; 2], b0: [f64; 2], b1: [f64; 2]) -> f64 {
    if segments_intersect(a0, a1, b0, b1) {
        return 0.0;
    }
    let d = [
        point_segment_distance(a0, b0, b1),
        point_segment_distance(a1, b0, b1),
        point_segment_distance(b0, a0, a1),
        point_segment_distance(b1, a0, a1),
    ];
    d[0].min(d[1]).min(d[2]).min(d[3])
}

fn segments_intersect(a0: [f64; 2], a1: [f64; 2], b0: [f64; 2], b1: [f64; 2]) -> bool {
    let cross = |o: [f64; 2], p: [f64; 2], q: [f64; 2]| {
        (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0])
    };
    let d1 = cross(b0, b1, a0);
    let d2 = cross(b0, b1, a1);
    let d3 = cross(a0, a1, b0);
    let d4 = cross(a0, a1, b1);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

fn point_segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let denom = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if denom <= 0.0 {
        0.0
    } else {
        ((ap[0] * ab[0] + ap[1] * ab[1]) / denom).clamp(0.0, 1.0)
    };
    let dx = a[0] + ab[0] * t - p[0];
    let dz = a[1] + ab[1] * t - p[1];
    sqrt(dx * dx + dz * dz)
}

/// Remove `[lo, hi]` from a sorted list of allowed intervals.
pub fn subtract_interval(allowed: &[(i64, i64)], lo: i64, hi: i64) -> Vec<(i64, i64)> {
    let mut result = Vec::with_capacity(allowed.len() + 1);
    for &(start, end) in allowed {
        if hi <= start || lo >= end {
            result.push((start, end));
            continue;
        }
        if lo > start {
            result.push((start, lo.min(end)));
        }
        if hi < end {
            result.push((hi.max(start), end));
        }
    }
    result.retain(|(a, b)| b > a);
    result
}

/// Every other element that shares a junction with `element_id`: its footprint
/// intersects, its run shares an endpoint, or one trims the other.
///
/// Undo uses this to widen the conflict scope. Removing one half of a junction
/// silently changes the other half's masonry, so a later edit to either side
/// must block the undo (architecture section 15.3).
pub fn junction_partners(
    document: &CastleDocument,
    terrain: &SiteTerrain,
    element_id: ElementId,
) -> BTreeSet<ElementId> {
    let mut partners = BTreeSet::new();
    let Some(element) = document.get(element_id) else {
        return partners;
    };
    for other in document.elements() {
        if other.id() == element_id {
            continue;
        }
        if elements_form_junction(terrain, element, other) {
            partners.insert(other.id());
        }
    }
    partners
}

/// Slack added to the touching reach so hand-dragged endpoints that leave a
/// few hundred millimetres of mortar still count as the same junction. Mirrors
/// the `750 mm` slack used by [`blocked_by_wall`].
const JUNCTION_SLACK_MM: f64 = 750.0;

fn elements_form_junction(terrain: &SiteTerrain, a: &Element, b: &Element) -> bool {
    match (a, b) {
        (Element::BatteredWall(a), Element::BatteredWall(b)) => walls_form_junction(a, b),
        (Element::RoundTower(tower), Element::BatteredWall(wall))
        | (Element::BatteredWall(wall), Element::RoundTower(tower)) => {
            tower_and_wall_form_junction(terrain, tower, wall)
        }
        (Element::RoundTower(a), Element::RoundTower(b)) => {
            let dx = a.centre_mm[0] as i128 - b.centre_mm[0] as i128;
            let dz = a.centre_mm[1] as i128 - b.centre_mm[1] as i128;
            let reach = i128::from(a.radius_mm) + i128::from(b.radius_mm);
            dx * dx + dz * dz <= reach * reach
        }
        (Element::Gatehouse(gatehouse), Element::BatteredWall(wall))
        | (Element::BatteredWall(wall), Element::Gatehouse(gatehouse)) => gatehouse
            .flank_towers()
            .iter()
            .any(|tower| tower_and_wall_form_junction(terrain, tower, wall)),
        (Element::Gatehouse(a), Element::Gatehouse(b)) => a.flank_towers().iter().any(|left| {
            b.flank_towers()
                .iter()
                .any(|right| towers_touch(left, right))
        }),
        (Element::Gatehouse(gatehouse), Element::RoundTower(tower))
        | (Element::RoundTower(tower), Element::Gatehouse(gatehouse)) => gatehouse
            .flank_towers()
            .iter()
            .any(|flank| towers_touch(flank, tower)),
        (Element::Ditch(_), _) | (_, Element::Ditch(_)) => false,
    }
}

fn towers_touch(a: &RoundTower, b: &RoundTower) -> bool {
    let dx = a.centre_mm[0] as i128 - b.centre_mm[0] as i128;
    let dz = a.centre_mm[1] as i128 - b.centre_mm[1] as i128;
    let reach = i128::from(a.radius_mm) + i128::from(b.radius_mm);
    dx * dx + dz * dz <= reach * reach
}

fn walls_form_junction(a: &BatteredWall, b: &BatteredWall) -> bool {
    if endpoints_touch(a.start_mm, b.start_mm)
        || endpoints_touch(a.start_mm, b.end_mm)
        || endpoints_touch(a.end_mm, b.start_mm)
        || endpoints_touch(a.end_mm, b.end_mm)
    {
        return true;
    }
    let reach = (a.thickness_mm.max(0) + b.thickness_mm.max(0)) as f64 * 0.5 + JUNCTION_SLACK_MM;
    segment_distance(
        [a.start_mm[0] as f64, a.start_mm[1] as f64],
        [a.end_mm[0] as f64, a.end_mm[1] as f64],
        [b.start_mm[0] as f64, b.start_mm[1] as f64],
        [b.end_mm[0] as f64, b.end_mm[1] as f64],
    ) <= reach
}

fn tower_and_wall_form_junction(
    terrain: &SiteTerrain,
    tower: &RoundTower,
    wall: &BatteredWall,
) -> bool {
    let (foundation_y, _, _) = tower_elevations(tower, terrain);
    let tower_reach = tower_outer_radius_at(tower, terrain, foundation_y).max(0) as f64;
    let wall_reach = wall_half_width_at(wall, terrain, foundation_y).max(0) as f64;
    let distance = point_segment_distance(
        [tower.centre_mm[0] as f64, tower.centre_mm[1] as f64],
        [wall.start_mm[0] as f64, wall.start_mm[1] as f64],
        [wall.end_mm[0] as f64, wall.end_mm[1] as f64],
    );
    distance <= tower_reach + wall_reach + JUNCTION_SLACK_MM
}

/// Priority ordering for a pair of element ids, used by tests and tooling.
pub fn higher_priority(document: &CastleDocument, a: ElementId, b: ElementId) -> Option<ElementId> {
    let ea = document.get(a)?;
    let eb = document.get(b)?;
    if outranks(ea, eb) {
        Some(a)
    } else if outranks(eb, ea) {
        Some(b)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::BatteredWall;

    #[test]
    fn subtract_interval_splits_and_clips() {
        let allowed = subtract_interval(&[(0, 100)], 40, 60);
        assert_eq!(allowed, vec![(0, 40), (60, 100)]);

        let allowed = subtract_interval(&[(0, 100)], -20, 30);
        assert_eq!(allowed, vec![(30, 100)]);

        let allowed = subtract_interval(&[(0, 100)], 70, 200);
        assert_eq!(allowed, vec![(0, 70)]);

        assert!(subtract_interval(&[(0, 100)], 0, 100).is_empty());
    }

    #[test]
    fn tower_outranks_wall_and_lower_id_wins_ties() {
        let mut doc = CastleDocument::new();
        doc.add_wall(BatteredWall::new(
            1,
            "wall",
            [0, 0],
            [10_000, 0],
            9_000,
            2_000,
        ));
        doc.add_tower(RoundTower::new(2, "tower", [8_000, 0], 3_000, 12_000));
        doc.add_wall(BatteredWall::new(
            3,
            "wall 2",
            [0, 5_000],
            [10_000, 5_000],
            9_000,
            2_000,
        ));
        assert_eq!(higher_priority(&doc, 1, 2), Some(2));
        assert_eq!(higher_priority(&doc, 1, 3), Some(1));
    }

    #[test]
    fn non_touching_walls_are_not_trimmed() {
        let terrain = SiteTerrain::m0a_promontory();
        let mut doc = CastleDocument::new();
        let b = BatteredWall::new(1, "b", [0, 0], [10_000, 0], 9_000, 2_000);
        let a = BatteredWall::new(2, "a", [12_000, -5_000], [12_000, 5_000], 9_000, 2_000);
        doc.add_wall(b.clone());
        doc.add_wall(a.clone());

        let d0 = -a.thickness_mm / 2;
        let d1 = a.thickness_mm / 2;
        let intervals =
            wall_course_intervals(&doc, &a, d0, d1, d0, d1, 0, a.length_mm(), 0, 450, &terrain);
        assert_eq!(
            intervals,
            vec![(0, a.length_mm())],
            "wall A should keep its full run"
        );
    }

    #[test]
    fn wall_half_width_shrinks_with_batter_and_drops_past_the_plinth() {
        let terrain = SiteTerrain::m0a_promontory();
        let wall = BatteredWall::new(1, "wall", [0, 0], [10_000, 0], 9_000, 2_000);
        let (foundation, plinth_top, _top) = wall_elevations(&wall, &terrain);
        let low = wall_half_width_at(&wall, &terrain, plinth_top - 10);
        let high = wall_half_width_at(&wall, &terrain, plinth_top + 10);
        assert!(low > high, "plinth should step the face out, then in");
        assert!(
            wall_half_width_at(&wall, &terrain, foundation + 9_000)
                < wall_half_width_at(&wall, &terrain, foundation + 100),
            "batter should recede with height"
        );
    }
}
