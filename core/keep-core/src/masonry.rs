//! Procedural masonry layout.
//!
//! Courses are horizontal world-space planes whose base steps with the terrain
//! and whose plinth courses project outward. Blocks are quantised prisms in
//! integer millimetres. Wall blocks span the full thickness; tower blocks span
//! an annular sector, so both conform to batter and curvature without gaps.
//!
//! Every block carries the data the renderer needs for the construction show
//! and weathering: a deterministic `build_index`, `course_index`, `age`,
//! `tint` and `jitter_seed`.
//!
//! Joints are solved once per joint, not once per block side, so neighbouring
//! stones share an exact plane and no daylight shows between them.

use crate::document::{
    BatteredWall, CastleDocument, Element, ElementKind, RoundTower, CASTLE_SEED,
};
use crate::hash01;
use crate::terrain::SiteTerrain;
use libm::{cos, round, sin, sqrt};

/// Face slots on a block, in the order used by `Block::expose`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockFace {
    Bottom = 0,
    Top = 1,
    Front = 2,
    Back = 3,
    End0 = 4,
    End1 = 5,
}

/// One quantised stone. Corner winding:
///
/// ```text
///       4----5          top
///      /|   /|
///     7----6 |
///     | 0--|-1          bottom
///     |/   |/
///     3----2
/// ```
///
/// `0..3` are the bottom ring, `4..7` the top ring; `0->1` runs along the
/// course (the `a` axis) and `0->3` crosses the thickness (the `d` axis).
#[derive(Clone, Debug)]
pub struct Block {
    pub element_id: u64,
    pub kind: ElementKind,
    pub course_index: i32,
    /// Position along the run / angle step, used only for canonical sorting.
    pub run_pos_mm: i64,
    pub build_index: u64,
    pub corners_mm: [[i64; 3]; 8],
    pub block_width_mm: i64,
    pub block_height_mm: i64,
    pub tint: f32,
    pub age: f32,
    pub jitter_seed: f32,
    /// `[bottom, top, front, back, end0, end1]`
    pub expose: [bool; 6],
}

impl Block {
    pub fn centre_mm(&self) -> [f64; 3] {
        let mut c = [0.0f64; 3];
        for corner in &self.corners_mm {
            c[0] += corner[0] as f64;
            c[1] += corner[1] as f64;
            c[2] += corner[2] as f64;
        }
        [c[0] / 8.0, c[1] / 8.0, c[2] / 8.0]
    }

    /// Corner indices for a face, in a stable order.
    ///
    /// All vertical side faces are ordered bottom-left, bottom-right, top-right,
    /// top-left as seen from outside, so `u` runs horizontally along the course
    /// (or across the thickness on end faces) and `v` runs bottom-to-top. The
    /// right-hand normal points out of the stone; `push_quad` reverses the
    /// triangles because Godot treats clockwise winding as front-facing.
    pub fn face_corners(face: BlockFace) -> [usize; 4] {
        match face {
            BlockFace::Bottom => [0, 1, 2, 3],
            BlockFace::Top => [4, 7, 6, 5],
            BlockFace::Front => [1, 0, 4, 5],
            BlockFace::Back => [3, 2, 6, 7],
            BlockFace::End0 => [0, 3, 7, 4],
            BlockFace::End1 => [2, 1, 5, 6],
        }
    }

    pub fn face_enabled(&self, face: BlockFace) -> bool {
        self.expose[face as usize]
    }
}

/// Generate the complete block list for a document, in canonical order.
pub fn generate_blocks(document: &CastleDocument, terrain: &SiteTerrain) -> Vec<Block> {
    let mut blocks = Vec::new();
    for element in document.elements() {
        match element {
            Element::BatteredWall(wall) => layout_wall(wall, terrain, &mut blocks),
            Element::RoundTower(tower) => layout_tower(tower, terrain, &mut blocks),
        }
    }

    // Canonical build order: course first (as a real wall rises), then stable
    // element and run position. This makes `build_index` independent of map
    // insertion order and of the order elements happened to be generated in.
    blocks.sort_by_key(|b| (b.course_index, b.element_id, b.run_pos_mm));
    for (index, block) in blocks.iter_mut().enumerate() {
        block.build_index = index as u64;
    }
    blocks
}

/// Block tint spread across warm limestone to cool grey. The shader uses this
/// as a variation factor, so a narrow band around 1.0 would read as flat.
fn stone_tint(hash: f32) -> f32 {
    0.15 + 0.75 * hash
}

fn layout_wall(wall: &BatteredWall, terrain: &SiteTerrain, out: &mut Vec<Block>) {
    let dx = (wall.end_mm[0] - wall.start_mm[0]) as f64;
    let dz = (wall.end_mm[1] - wall.start_mm[1]) as f64;
    let len = sqrt(dx * dx + dz * dz);
    if len < 1.0 {
        return;
    }
    let dir = [dx / len, dz / len];
    // Right-hand plan normal: the d axis across the wall thickness.
    let right = [-dir[1], dir[0]];
    let length_mm = round(len) as i64;

    let (ground_min, ground_max) = ground_range_along(wall.start_mm, dir, len, terrain);
    let foundation_y = ground_min - 1_200;
    let plinth_top_y = ground_max + wall.plinth_height_mm;
    let top_y = ground_max + wall.height_mm;
    let total_height = (top_y - foundation_y).max(1);

    let course_nominal = wall.course_height_mm.max(150);
    let mut course_index = 0i32;
    let mut y0 = foundation_y;
    let element_seed = wall.seed ^ CASTLE_SEED;

    // Running bond: alternate courses shift by half a nominal block.
    let nominal_block = 950i64;

    while y0 < top_y {
        let jitter = (hash01(element_seed, i64::from(course_index), 0, 0) - 0.5) * 40.0;
        let mut y1 = y0 + course_nominal + jitter as i64;
        if y1 > top_y || top_y - y1 < course_nominal / 2 {
            y1 = top_y;
        }
        if y1 <= y0 {
            break;
        }

        let mid_y = (y0 + y1) / 2;
        let t = ((mid_y - foundation_y) as f64 / total_height as f64).clamp(0.0, 1.0);
        let batter_inset = round(wall.batter_mm as f64 * t) as i64;
        let is_plinth = y0 < plinth_top_y;
        let plinth = if is_plinth {
            wall.plinth_projection_mm
        } else {
            0
        };
        let expose_top = y1 >= top_y || (is_plinth && y1 >= plinth_top_y);

        let d0 = -wall.thickness_mm / 2 + batter_inset / 2 - plinth;
        let mut d1 = wall.thickness_mm / 2 - batter_inset / 2 + plinth;
        if d1 <= d0 {
            d1 = d0 + 100;
        }

        let bond = if course_index % 2 == 0 {
            0
        } else {
            nominal_block / 2
        };

        // Solve every joint once for this course. Blocks on either side of a
        // joint read the same position, so the shared plane is exact.
        let mut joints_mm = Vec::with_capacity(32);
        joints_mm.push(-bond);
        let mut a = -bond;
        let mut column = 0i32;
        while a < length_mm {
            let vary =
                (hash01(element_seed, i64::from(course_index), i64::from(column), 7) - 0.5) * 420.0;
            let mut next = a + nominal_block + vary as i64;
            if next <= a + 250 {
                next = a + 500;
            }
            a = next;
            column += 1;
            // Jitter is applied to the joint itself, not to either block side.
            let joint_jitter = stone_jitter(element_seed, course_index, column, 1);
            joints_mm.push(a + joint_jitter);
        }
        // Joint jitter can leave the last joint a few millimetres short of the
        // wall end; close it exactly so the run is never truncated.
        if let Some(last) = joints_mm.last_mut() {
            *last = (*last).max(length_mm);
        }

        let mut joint_index = 0usize;
        while joint_index + 1 < joints_mm.len() {
            let a_clamped = joints_mm[joint_index].max(0);
            if a_clamped >= length_mm {
                break;
            }
            let b_clamped = joints_mm[joint_index + 1].min(length_mm);
            let width = b_clamped - a_clamped;
            if width <= 0 {
                joint_index += 1;
                continue;
            }

            if width < 240 {
                // Extend the previous stone cleanly to the wall end rather than
                // dropping a sliver that would leave an open vertical joint.
                if let Some(previous) = out
                    .last_mut()
                    .filter(|b| b.element_id == wall.id && b.course_index == course_index)
                {
                    extend_block_along(previous, dir, length_mm - joints_mm[joint_index]);
                    previous.block_width_mm = length_mm - previous.run_pos_mm;
                    previous.expose[BlockFace::End1 as usize] = true;
                } else {
                    push_wall_block(
                        out,
                        wall,
                        dir,
                        right,
                        element_seed,
                        course_index,
                        expose_top,
                        a_clamped,
                        length_mm,
                        d0,
                        d1,
                        y0,
                        y1,
                        t,
                        terrain,
                    );
                }
                break;
            }

            push_wall_block(
                out,
                wall,
                dir,
                right,
                element_seed,
                course_index,
                expose_top,
                a_clamped,
                b_clamped,
                d0,
                d1,
                y0,
                y1,
                t,
                terrain,
            );

            joint_index += 1;
        }

        course_index += 1;
        y0 = y1;
    }
}

#[allow(clippy::too_many_arguments)]
fn push_wall_block(
    out: &mut Vec<Block>,
    wall: &BatteredWall,
    dir: [f64; 2],
    right: [f64; 2],
    element_seed: u64,
    course_index: i32,
    expose_top: bool,
    a0: i64,
    a1: i64,
    d0: i64,
    d1: i64,
    y0: i64,
    y1: i64,
    height_fraction: f64,
    terrain: &SiteTerrain,
) {
    let column = a0 / 950;
    let j_d0 = stone_jitter(element_seed, course_index, column as i32, 3);
    let j_d1 = stone_jitter(element_seed, course_index, column as i32, 4);

    let corners = wall_corners(wall, dir, right, a0, a1, d0 + j_d0, d1 + j_d1, y0, y1);

    if is_buried(&corners, terrain) {
        return;
    }

    let width = (a1 - a0).max(1);
    let height = (y1 - y0).max(1);
    out.push(Block {
        element_id: wall.id,
        kind: ElementKind::BatteredWall,
        course_index,
        run_pos_mm: a0,
        build_index: 0,
        corners_mm: corners,
        block_width_mm: width,
        block_height_mm: height,
        tint: stone_tint(hash01(element_seed, course_index as i64, column, 11)),
        age: (0.55 + 0.45 * height_fraction as f32).min(1.0),
        jitter_seed: hash01(element_seed, course_index as i64, column, 12),
        expose: [
            false,
            expose_top,
            true,
            true,
            a0 <= 1,
            a1 >= wall.length_mm() - 1,
        ],
    });
}

/// Push the block's end face (corners 1, 2, 5, 6) along the run direction.
fn extend_block_along(block: &mut Block, dir: [f64; 2], delta_mm: i64) {
    if delta_mm <= 0 {
        return;
    }
    let dx = round(dir[0] * delta_mm as f64) as i64;
    let dz = round(dir[1] * delta_mm as f64) as i64;
    for index in [1usize, 2, 5, 6] {
        block.corners_mm[index][0] += dx;
        block.corners_mm[index][2] += dz;
    }
}

/// True when the block's whole top is buried below the ground surface.
fn is_buried(corners: &[[i64; 3]; 8], terrain: &SiteTerrain) -> bool {
    let top = corners[6][1].max(corners[4][1]).max(corners[5][1]);
    let cx = (corners[0][0] + corners[6][0]) / 2;
    let cz = (corners[0][2] + corners[6][2]) / 2;
    top <= terrain.height_mm(cx, cz) - 300
}

#[allow(clippy::too_many_arguments)]
fn wall_corners(
    wall: &BatteredWall,
    dir: [f64; 2],
    right: [f64; 2],
    a0: i64,
    a1: i64,
    d0: i64,
    d1: i64,
    y0: i64,
    y1: i64,
) -> [[i64; 3]; 8] {
    let base_x = wall.start_mm[0] as f64;
    let base_z = wall.start_mm[1] as f64;
    let point = |a: i64, d: i64, y: i64| -> [i64; 3] {
        let x = base_x + dir[0] * a as f64 + right[0] * d as f64;
        let z = base_z + dir[1] * a as f64 + right[1] * d as f64;
        [round(x) as i64, y, round(z) as i64]
    };
    [
        point(a0, d0, y0),
        point(a1, d0, y0),
        point(a1, d1, y0),
        point(a0, d1, y0),
        point(a0, d0, y1),
        point(a1, d0, y1),
        point(a1, d1, y1),
        point(a0, d1, y1),
    ]
}

fn layout_tower(tower: &RoundTower, terrain: &SiteTerrain, out: &mut Vec<Block>) {
    let radius = tower.radius_mm.max(500);
    let thickness = tower.thickness_mm.clamp(300, radius);
    let (ground_min, ground_max) = ground_range_tower(tower, terrain);
    let foundation_y = ground_min - 1_200;
    let plinth_top_y = ground_max + tower.plinth_height_mm;
    let top_y = ground_max + tower.height_mm;
    let total_height = (top_y - foundation_y).max(1);
    let element_seed = tower.seed ^ CASTLE_SEED.rotate_left(9);

    // Ring block count is fixed over the height so vertical joints line up and
    // the running bond reads around the curve.
    let circumference = 2.0 * std::f64::consts::PI * radius as f64;
    let ring_count = ((circumference / 1_050.0).round() as i32).max(8);
    let step = std::f64::consts::TAU / ring_count as f64;

    let course_nominal = tower.course_height_mm.max(150);
    let mut course_index = 0i32;
    let mut y0 = foundation_y;

    while y0 < top_y {
        let jitter = (hash01(element_seed, i64::from(course_index), 1, 0) - 0.5) * 40.0;
        let mut y1 = y0 + course_nominal + jitter as i64;
        if y1 > top_y || top_y - y1 < course_nominal / 2 {
            y1 = top_y;
        }
        if y1 <= y0 {
            break;
        }

        let mid_y = (y0 + y1) / 2;
        let t = ((mid_y - foundation_y) as f64 / total_height as f64).clamp(0.0, 1.0);
        let batter_inset = round(tower.batter_mm as f64 * t) as i64;
        let is_plinth = y0 < plinth_top_y;
        let plinth = if is_plinth {
            tower.plinth_projection_mm
        } else {
            0
        };
        let expose_top = y1 >= top_y || (is_plinth && y1 >= plinth_top_y);
        let r_outer = (radius - batter_inset / 2 + plinth).max(400);
        let r_inner = (radius - thickness - batter_inset / 2 - plinth).max(80);

        let bond = if course_index % 2 == 0 {
            0.0
        } else {
            step * 0.5
        };

        // One shared angle per joint. Converting millimetre jitter into
        // radians requires dividing by the radius, not scaling by 0.001.
        let joint_angle = |joint: i32| -> f64 {
            let jitter_mm = stone_jitter(element_seed, course_index, joint, 1) as f64;
            bond + step * joint as f64 + jitter_mm / radius as f64
        };

        for j in 0..ring_count {
            let a0 = joint_angle(j);
            let a1 = joint_angle(j + 1);
            let jr = stone_jitter(element_seed, course_index, j, 3);
            let ro = (r_outer + jr).max(300);
            let ri = (r_inner + jr / 2).max(60);

            let corners = tower_corners(tower.centre_mm, a0, a1, ro, ri, y0, y1);
            if is_buried(&corners, terrain) {
                continue;
            }

            let width = round(ro as f64 * step) as i64;
            out.push(Block {
                element_id: tower.id,
                kind: ElementKind::RoundTower,
                course_index,
                run_pos_mm: j as i64,
                build_index: 0,
                corners_mm: corners,
                block_width_mm: width.max(1),
                block_height_mm: (y1 - y0).max(1),
                tint: stone_tint(hash01(element_seed, course_index as i64, j as i64, 11)),
                age: (0.55 + 0.45 * t as f32).min(1.0),
                jitter_seed: hash01(element_seed, course_index as i64, j as i64, 12),
                expose: [false, expose_top, true, true, false, false],
            });
        }

        course_index += 1;
        y0 = y1;
    }
}

#[allow(clippy::too_many_arguments)]
fn tower_corners(
    centre_mm: [i64; 2],
    a0: f64,
    a1: f64,
    r_outer: i64,
    r_inner: i64,
    y0: i64,
    y1: i64,
) -> [[i64; 3]; 8] {
    let cx = centre_mm[0] as f64;
    let cz = centre_mm[1] as f64;
    let p = |angle: f64, r: i64, y: i64| -> [i64; 3] {
        let x = cx + cos(angle) * r as f64;
        let z = cz + sin(angle) * r as f64;
        [round(x) as i64, y, round(z) as i64]
    };
    [
        p(a0, r_outer, y0),
        p(a1, r_outer, y0),
        p(a1, r_inner, y0),
        p(a0, r_inner, y0),
        p(a0, r_outer, y1),
        p(a1, r_outer, y1),
        p(a1, r_inner, y1),
        p(a0, r_inner, y1),
    ]
}

fn stone_jitter(seed: u64, course: i32, column: i32, slot: i64) -> i64 {
    ((hash01(seed, course as i64, column as i64, 100 + slot) - 0.5) * 14.0).round() as i64
}

fn ground_range_along(
    start_mm: [i64; 2],
    dir: [f64; 2],
    len: f64,
    terrain: &SiteTerrain,
) -> (i64, i64) {
    const SAMPLES: i32 = 24;
    let mut min_h = i64::MAX;
    let mut max_h = i64::MIN;
    for i in 0..=SAMPLES {
        let t = i as f64 / SAMPLES as f64;
        let x = start_mm[0] as f64 + dir[0] * len * t;
        let z = start_mm[1] as f64 + dir[1] * len * t;
        let h = terrain.height_mm(round(x) as i64, round(z) as i64);
        min_h = min_h.min(h);
        max_h = max_h.max(h);
    }
    (min_h, max_h)
}

fn ground_range_tower(tower: &RoundTower, terrain: &SiteTerrain) -> (i64, i64) {
    let mut min_h = terrain.height_mm(tower.centre_mm[0], tower.centre_mm[1]);
    let mut max_h = min_h;
    for i in 0..12 {
        let angle = std::f64::consts::TAU * i as f64 / 12.0;
        let x = tower.centre_mm[0] as f64 + cos(angle) * tower.radius_mm as f64;
        let z = tower.centre_mm[1] as f64 + sin(angle) * tower.radius_mm as f64;
        let h = terrain.height_mm(round(x) as i64, round(z) as i64);
        min_h = min_h.min(h);
        max_h = max_h.max(h);
    }
    (min_h, max_h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::m0a_showcase_document;

    #[test]
    fn blocks_have_monotonic_build_indices_and_courses() {
        let terrain = SiteTerrain::m0a_promontory();
        let doc = m0a_showcase_document();
        let blocks = generate_blocks(&doc, &terrain);
        assert!(!blocks.is_empty());
        for (i, block) in blocks.iter().enumerate() {
            assert_eq!(block.build_index, i as u64);
        }
        let mut last_course = i32::MIN;
        for block in &blocks {
            assert!(block.course_index >= last_course);
            last_course = block.course_index;
        }
    }

    #[test]
    fn every_block_has_positive_volume() {
        let terrain = SiteTerrain::m0a_promontory();
        let doc = m0a_showcase_document();
        for block in generate_blocks(&doc, &terrain) {
            let [x0, y0, z0] = block.corners_mm[0];
            let [x1, y1, z1] = block.corners_mm[6];
            assert!(x1 != x0 || z1 != z0);
            assert!(y1 > y0);
        }
    }

    #[test]
    fn wall_adjacent_stones_share_joints_without_gaps() {
        let terrain = SiteTerrain::m0a_promontory();
        let doc = m0a_showcase_document();
        let blocks = generate_blocks(&doc, &terrain);
        // The M0a curtain runs along +X, so projecting onto the run axis is
        // just the x coordinate of the shared joint corners.
        let mut courses: std::collections::BTreeMap<i32, Vec<&Block>> =
            std::collections::BTreeMap::new();
        for block in &blocks {
            if block.kind == ElementKind::BatteredWall {
                courses.entry(block.course_index).or_default().push(block);
            }
        }
        assert!(!courses.is_empty());
        for (course, mut stones) in courses {
            stones.sort_by_key(|b| b.run_pos_mm);
            for pair in stones.windows(2) {
                let left = pair[0];
                let right = pair[1];
                // End face of the left stone (corners 1 and 5) and start face
                // of the right stone (corners 0 and 4) coincide on the joint.
                for (end, start) in [(1usize, 0usize), (5, 4)] {
                    let gap = (right.corners_mm[start][0] - left.corners_mm[end][0]).abs();
                    assert!(
                        gap <= 1,
                        "course {course}: {gap} mm gap between stones at x={}",
                        left.corners_mm[end][0]
                    );
                }
            }
        }
    }

    #[test]
    fn tower_adjacent_stones_share_joints_without_gaps() {
        use libm::{atan2, cos, sin};
        let terrain = SiteTerrain::m0a_promontory();
        let doc = m0a_showcase_document();
        let blocks = generate_blocks(&doc, &terrain);
        let tower = match doc.get(2).unwrap() {
            Element::RoundTower(t) => t,
            _ => unreachable!(),
        };
        let cx = tower.centre_mm[0] as f64;
        let cz = tower.centre_mm[1] as f64;
        let mut courses: std::collections::BTreeMap<i32, Vec<&Block>> =
            std::collections::BTreeMap::new();
        for block in &blocks {
            if block.kind == ElementKind::RoundTower {
                courses.entry(block.course_index).or_default().push(block);
            }
        }
        assert!(!courses.is_empty());
        let mut checked = 0;
        for (course, mut stones) in courses {
            stones.sort_by_key(|b| b.run_pos_mm);
            for pair in stones.windows(2) {
                let left = pair[0];
                let right = pair[1];
                // Compare the midpoints of each side's joint face. Both lie on
                // the shared joint ray; projecting their separation onto the
                // tangent cancels millimetre position quantisation.
                let midpoint = |a: [i64; 3], b: [i64; 3]| {
                    [
                        (a[0] as f64 + b[0] as f64) * 0.5 - cx,
                        (a[2] as f64 + b[2] as f64) * 0.5 - cz,
                    ]
                };
                let left_mid = midpoint(left.corners_mm[1], left.corners_mm[2]);
                let right_mid = midpoint(right.corners_mm[0], right.corners_mm[3]);
                let angle = atan2(left_mid[1], left_mid[0]);
                let tangent = [-sin(angle), cos(angle)];
                let gap_mm = ((right_mid[0] - left_mid[0]) * tangent[0]
                    + (right_mid[1] - left_mid[1]) * tangent[1])
                    .abs();
                assert!(
                    gap_mm <= 1.0,
                    "course {course}: {gap_mm} mm angular gap between stones"
                );
                checked += 1;
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn wall_free_end_is_extended_not_dropped() {
        let terrain = SiteTerrain::m0a_promontory();
        let doc = m0a_showcase_document();
        let wall = match doc.get(1).unwrap() {
            Element::BatteredWall(w) => w,
            _ => unreachable!(),
        };
        let blocks = generate_blocks(&doc, &terrain);
        let mut courses: std::collections::BTreeMap<i32, Vec<&Block>> =
            std::collections::BTreeMap::new();
        for block in &blocks {
            if block.kind == ElementKind::BatteredWall {
                courses.entry(block.course_index).or_default().push(block);
            }
        }
        // The lowest courses are legitimately culled where the ground rises,
        // so check the topmost course, which is fully exposed.
        let (_, mut stones) = courses.pop_last().unwrap();
        stones.sort_by_key(|b| b.run_pos_mm);
        let last = stones.last().unwrap();
        assert!(
            last.run_pos_mm + last.block_width_mm >= wall.length_mm() - 1,
            "wall stops {} mm short of its end",
            wall.length_mm() - (last.run_pos_mm + last.block_width_mm)
        );
        assert!(last.expose[BlockFace::End1 as usize]);
    }

    #[test]
    fn vertical_faces_run_horizontal_then_up_with_outward_winding() {
        let terrain = SiteTerrain::m0a_promontory();
        let doc = m0a_showcase_document();
        let block = generate_blocks(&doc, &terrain)
            .into_iter()
            .find(|b| b.kind == ElementKind::BatteredWall)
            .expect("a wall block");
        let centre = block.centre_mm();
        for face in [
            BlockFace::Front,
            BlockFace::Back,
            BlockFace::End0,
            BlockFace::End1,
        ] {
            let ids = Block::face_corners(face);
            let p: Vec<[f64; 3]> = ids
                .iter()
                .map(|i| {
                    [
                        block.corners_mm[*i][0] as f64,
                        block.corners_mm[*i][1] as f64,
                        block.corners_mm[*i][2] as f64,
                    ]
                })
                .collect();
            // u runs horizontally along the bottom edge.
            assert_eq!(p[0][1], p[1][1], "{face:?}: u edge is not horizontal");
            assert!(
                (p[0][0] - p[1][0]).abs() + (p[0][2] - p[1][2]).abs() > 0.0,
                "{face:?}: u edge has no horizontal run"
            );
            // v runs bottom-to-top.
            assert!(p[2][1] > p[1][1], "{face:?}: v edge does not go up");
            assert_eq!(p[2][1], p[3][1], "{face:?}: top edge is not horizontal");

            // Right-hand winding points away from the block centre.
            let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
            let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
            let normal = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let face_centre = [
                (p[0][0] + p[1][0] + p[2][0] + p[3][0]) * 0.25,
                (p[0][1] + p[1][1] + p[2][1] + p[3][1]) * 0.25,
                (p[0][2] + p[1][2] + p[2][2] + p[3][2]) * 0.25,
            ];
            let outward = [
                face_centre[0] - centre[0],
                face_centre[1] - centre[1],
                face_centre[2] - centre[2],
            ];
            let dot = normal[0] * outward[0] + normal[1] * outward[1] + normal[2] * outward[2];
            assert!(dot > 0.0, "{face:?}: normal points inward ({dot})");
        }
    }
}
