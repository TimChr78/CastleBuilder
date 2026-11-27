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
    BatteredWall, CastleDocument, Element, ElementKind, Opening, RoundTower, CASTLE_SEED,
};
use crate::hash01;
use crate::junction;
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

/// What role a block plays in the masonry. Most stones are ordinary
/// [`BlockRole::Masonry`]; the trim and void solver tags frame stones so they
/// can be highlighted in debug views and checked by tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlockRole {
    /// Ordinary wall or tower stone.
    Masonry,
    /// Sloped, stepped stone lining an embrasure splay.
    Jamb,
    /// Stone directly under an opening (the embrasure floor).
    Sill,
    /// Stone directly over an opening.
    Lintel,
    /// End stone dressed where a wall was trimmed at a junction seam.
    SeamQuoin,
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
    pub role: BlockRole,
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
            Element::BatteredWall(wall) => layout_wall(wall, document, terrain, &mut blocks),
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

fn layout_wall(
    wall: &BatteredWall,
    document: &CastleDocument,
    terrain: &SiteTerrain,
    out: &mut Vec<Block>,
) {
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

    // Openings attached to this wall, resolved once into wall-local cuts.
    let cuts: Vec<OpeningCut> = document
        .openings_for_wall(wall.id)
        .map(|opening| OpeningCut::resolve(wall, opening, terrain))
        .collect();
    let mut first_void_course: Vec<Option<i32>> = vec![None; cuts.len()];
    let mut last_void_course: Vec<Option<i32>> = vec![None; cuts.len()];
    // Openings that overlap another opening are never carved: a partially
    // framed hole is worse than a missing loop.
    let mut cut_overlaps = vec![false; cuts.len()];
    for i in 0..cuts.len() {
        for j in (i + 1)..cuts.len() {
            let a_lo = cuts[i].a_centre - cuts[i].inner_half;
            let a_hi = cuts[i].a_centre + cuts[i].inner_half;
            let b_lo = cuts[j].a_centre - cuts[j].inner_half;
            let b_hi = cuts[j].a_centre + cuts[j].inner_half;
            if a_hi > b_lo && b_hi > a_lo {
                cut_overlaps[i] = true;
                cut_overlaps[j] = true;
            }
        }
    }

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

        // Priority-ordered junction trimming: wall stones may only exist in
        // these run intervals once towers and higher-priority walls have taken
        // their volume. The higher-priority wall of an angled corner is
        // extended past the shared endpoint so the outer mitre stays solid.
        let (extend_start, extend_end) =
            junction::wall_corner_extensions(document, wall, y0, y1, terrain);
        let run_lo = -extend_start;
        let run_hi = length_mm + extend_end;
        let intervals = junction::wall_course_intervals(
            document, wall, d0, d1, run_lo, run_hi, y0, y1, terrain,
        );
        let mut solid = intervals.clone();
        let mut active_cuts: Vec<usize> = Vec::new();
        for (index, cut) in cuts.iter().enumerate() {
            if !cut.is_void_course(mid_y) || cut_overlaps[index] {
                continue;
            }
            let band_lo = cut.a_centre - cut.inner_half;
            let band_hi = cut.a_centre + cut.inner_half;
            // Only carve when the *whole* splayed band lies inside one
            // allowed post-junction run, so an opening can never leave an
            // unframed hole at a seam.
            let contained = intervals
                .iter()
                .any(|&(start, end)| start <= band_lo && end >= band_hi);
            if !contained {
                continue;
            }
            solid = junction::subtract_interval(&solid, band_lo, band_hi);
            active_cuts.push(index);
            first_void_course[index].get_or_insert(course_index);
            last_void_course[index] = Some(course_index);
        }

        let full_untouched = intervals.len() == 1
            && intervals[0] == (0, length_mm)
            && solid.len() == 1
            && solid[0] == (0, length_mm)
            && active_cuts.is_empty();

        if full_untouched {
            layout_full_course(
                out,
                wall,
                dir,
                right,
                element_seed,
                course_index,
                expose_top,
                length_mm,
                d0,
                d1,
                y0,
                y1,
                t,
                terrain,
                nominal_block,
            );
        } else {
            // Boundaries where an opening jamb meets an ordinary stone must not
            // emit a coincident face (that would z-fight inside the wall).
            let mut jamb_edges = Vec::new();
            for &index in &active_cuts {
                let cut = &cuts[index];
                jamb_edges.push(cut.a_centre - cut.inner_half);
                jamb_edges.push(cut.a_centre + cut.inner_half);
            }

            for &index in &active_cuts {
                let cut = &cuts[index];
                emit_opening_jambs(
                    out,
                    wall,
                    dir,
                    right,
                    element_seed,
                    course_index,
                    expose_top,
                    y0,
                    y1,
                    t,
                    terrain,
                    cut,
                    &intervals,
                    d0,
                    d1,
                );
            }

            for &(start, end) in &solid {
                if end <= start {
                    continue;
                }
                let expose_start = boundary_exposed(start, length_mm, &intervals, &jamb_edges);
                let expose_end = boundary_exposed(end, length_mm, &intervals, &jamb_edges);
                layout_course_run(
                    out,
                    wall,
                    dir,
                    right,
                    element_seed,
                    course_index,
                    expose_top,
                    start,
                    end,
                    d0,
                    d1,
                    y0,
                    y1,
                    t,
                    terrain,
                    nominal_block,
                    expose_start,
                    expose_end,
                );
            }

            // Dress the stone that terminates in a round tower so its end
            // face follows the tower's circular curve instead of a flat chord.
            if let Some(seam) = junction::tower_seam(document, wall, d0, d1, y0, y1, terrain) {
                dress_tower_seam(out, wall, dir, right, course_index, d0, d1, y0, y1, seam);
            }
            // ...and the stone that meets a higher-priority wall so its end
            // face lies along the incumbent's battered face instead of a flat
            // run cut that leaves a triangular daylight gap.
            for seam in junction::wall_seams(document, wall, d0, d1, y0, y1, terrain) {
                dress_wall_seam(out, wall, dir, right, course_index, d0, d1, y0, y1, seam);
            }
        }

        course_index += 1;
        y0 = y1;
    }

    // Tag the stones immediately below and above each void as the sill and
    // lintel frame, so tooling can highlight the embrasure.
    for (index, cut) in cuts.iter().enumerate() {
        let (Some(first), Some(last)) = (first_void_course[index], last_void_course[index]) else {
            continue;
        };
        let band_lo = cut.a_centre - cut.inner_half;
        let band_hi = cut.a_centre + cut.inner_half;
        for block in out.iter_mut() {
            if block.element_id != wall.id || block.role != BlockRole::Masonry {
                continue;
            }
            let overlaps =
                block.run_pos_mm < band_hi && block.run_pos_mm + block.block_width_mm > band_lo;
            if !overlaps {
                continue;
            }
            if block.course_index == first - 1 {
                block.role = BlockRole::Sill;
                block.expose[BlockFace::Top as usize] = true;
            } else if block.course_index == last + 1 {
                block.role = BlockRole::Lintel;
                block.expose[BlockFace::Bottom as usize] = true;
            }
        }
    }
}

/// A resolved opening in a wall's local `(a, d, y)` frame.
#[derive(Clone, Debug)]
struct OpeningCut {
    /// Plan position along the run.
    a_centre: i64,
    /// Half-width of the outer slit.
    outer_half: i64,
    /// Half-width of the inner embrasure.
    inner_half: i64,
    /// Void bottom and top elevations.
    sill_y: i64,
    lintel_y: i64,
}

impl OpeningCut {
    fn resolve(wall: &BatteredWall, opening: &Opening, terrain: &SiteTerrain) -> Self {
        let length = wall.length_mm().max(1);
        let a_centre = round(opening.t_permille as f64 / 1000.0 * length as f64) as i64;
        let plan = opening.plan_point_mm(wall);
        let ground = terrain.height_mm(plan[0], plan[1]);
        let sill_y = ground + opening.sill_height_mm.max(0);
        let lintel_y = sill_y + opening.height_mm.max(200);
        let outer_half = (opening.outer_slit_mm.max(1) / 2).max(1);
        let inner_half = (opening
            .inner_splay_width_mm
            .max(opening.outer_slit_mm.max(1))
            / 2)
        .max(outer_half + 10);
        Self {
            a_centre,
            outer_half,
            inner_half,
            sill_y,
            lintel_y,
        }
    }

    fn is_void_course(&self, mid_y: i64) -> bool {
        mid_y >= self.sill_y && mid_y < self.lintel_y
    }

    /// Void half-width at normalised depth `s` (`0` outside, `1` inside).
    fn half_at(&self, s: f64) -> i64 {
        round(self.outer_half as f64 + (self.inner_half - self.outer_half) as f64 * s) as i64
    }
}

/// True when a run boundary should emit its end face: free wall ends and
/// junction seams do, but boundaries that butt into an opening jamb do not.
fn boundary_exposed(
    edge: i64,
    length_mm: i64,
    intervals: &[(i64, i64)],
    jamb_edges: &[i64],
) -> bool {
    if jamb_edges.contains(&edge) {
        return false;
    }
    if edge == 0 || edge == length_mm {
        return true;
    }
    intervals
        .iter()
        .any(|&(start, end)| start == edge || end == edge)
}

/// Original whole-course layout. Kept byte-for-byte compatible so documents
/// without junctions or openings keep their M0a layout hashes.
#[allow(clippy::too_many_arguments)]
fn layout_full_course(
    out: &mut Vec<Block>,
    wall: &BatteredWall,
    dir: [f64; 2],
    right: [f64; 2],
    element_seed: u64,
    course_index: i32,
    expose_top: bool,
    length_mm: i64,
    d0: i64,
    d1: i64,
    y0: i64,
    y1: i64,
    height_fraction: f64,
    terrain: &SiteTerrain,
    nominal_block: i64,
) {
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
                    height_fraction,
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
            height_fraction,
            terrain,
        );

        joint_index += 1;
    }
}

/// Lay out one arbitrary solid run interval, used after junction trimming.
#[allow(clippy::too_many_arguments)]
fn layout_course_run(
    out: &mut Vec<Block>,
    wall: &BatteredWall,
    dir: [f64; 2],
    right: [f64; 2],
    element_seed: u64,
    course_index: i32,
    expose_top: bool,
    run_start: i64,
    run_end: i64,
    d0: i64,
    d1: i64,
    y0: i64,
    y1: i64,
    height_fraction: f64,
    terrain: &SiteTerrain,
    nominal_block: i64,
    expose_start: bool,
    expose_end: bool,
) {
    if run_end <= run_start {
        return;
    }

    let mut joints = Vec::with_capacity(16);
    joints.push(run_start);
    let mut a = run_start;
    let mut column = (run_start / nominal_block) as i32;
    while a < run_end {
        let vary =
            (hash01(element_seed, i64::from(course_index), i64::from(column), 7) - 0.5) * 420.0;
        let mut next = a + nominal_block + vary as i64;
        if next <= a + 250 {
            next = a + 500;
        }
        a = next;
        column += 1;
        let joint_jitter = stone_jitter(element_seed, course_index, column, 1);
        joints.push(a + joint_jitter);
    }
    if let Some(last) = joints.last_mut() {
        *last = (*last).max(run_end);
    }

    // The bond shift only affects the interior joints; the run ends are fixed
    // by the junction solver so seams stay watertight.
    let mut joint_index = 0usize;
    while joint_index + 1 < joints.len() {
        let a0 = joints[joint_index].max(run_start);
        if a0 >= run_end {
            break;
        }
        let a1 = joints[joint_index + 1].min(run_end);
        let width = a1 - a0;
        if width <= 0 {
            joint_index += 1;
            continue;
        }

        if width < 240 {
            if let Some(previous) = out.last_mut().filter(|b| {
                b.element_id == wall.id
                    && b.course_index == course_index
                    && b.run_pos_mm >= run_start
            }) {
                extend_block_along(previous, dir, run_end - a0);
                previous.block_width_mm = run_end - previous.run_pos_mm;
                previous.expose[BlockFace::End1 as usize] = expose_end;
                previous.role = if expose_end && run_end != 0 && run_end != wall.length_mm() {
                    BlockRole::SeamQuoin
                } else {
                    previous.role
                };
            } else {
                let role = if run_start != 0 && run_start != wall.length_mm() && expose_start {
                    BlockRole::SeamQuoin
                } else {
                    BlockRole::Masonry
                };
                push_interval_block(
                    out,
                    wall,
                    dir,
                    right,
                    element_seed,
                    course_index,
                    expose_top,
                    a0,
                    run_end,
                    d0,
                    d1,
                    y0,
                    y1,
                    height_fraction,
                    terrain,
                    expose_start,
                    expose_end,
                    role,
                );
            }
            break;
        }

        let at_seam =
            (a0 == run_start && expose_start && run_start != 0 && run_start != wall.length_mm())
                || (a1 == run_end && expose_end && run_end != 0 && run_end != wall.length_mm());
        let role = if at_seam {
            BlockRole::SeamQuoin
        } else {
            BlockRole::Masonry
        };
        push_interval_block(
            out,
            wall,
            dir,
            right,
            element_seed,
            course_index,
            expose_top,
            a0,
            a1,
            d0,
            d1,
            y0,
            y1,
            height_fraction,
            terrain,
            expose_start && a0 == run_start,
            expose_end && a1 == run_end,
            role,
        );

        joint_index += 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn push_interval_block(
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
    expose_start: bool,
    expose_end: bool,
    role: BlockRole,
) {
    let column = a0 / 950;
    let j_d0 = stone_jitter(element_seed, course_index, column as i32, 3);
    let j_d1 = stone_jitter(element_seed, course_index, column as i32, 4);
    let corners = wall_corners(wall, dir, right, a0, a1, d0 + j_d0, d1 + j_d1, y0, y1);
    if is_buried(&corners, terrain) {
        return;
    }
    push_block(
        out,
        wall.id,
        ElementKind::BatteredWall,
        course_index,
        a0,
        corners,
        a1 - a0,
        y1 - y0,
        element_seed,
        column,
        height_fraction,
        [false, expose_top, true, true, expose_start, expose_end],
        role,
    );
}

/// Original whole-course block emitter, byte-for-byte compatible with M0a.
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

    push_block(
        out,
        wall.id,
        ElementKind::BatteredWall,
        course_index,
        a0,
        corners,
        (a1 - a0).max(1),
        (y1 - y0).max(1),
        element_seed,
        column,
        height_fraction,
        [
            false,
            expose_top,
            true,
            true,
            a0 <= 1,
            a1 >= wall.length_mm() - 1,
        ],
        BlockRole::Masonry,
    );
}

/// Emit the stepped splay jambs that narrow the embrasure from the wide inner
/// face down to the narrow outer slit. Each step is a proper trapezoidal prism
/// whose faces exactly match the void boundary, so the reveal is watertight.
#[allow(clippy::too_many_arguments)]
fn emit_opening_jambs(
    out: &mut Vec<Block>,
    wall: &BatteredWall,
    dir: [f64; 2],
    right: [f64; 2],
    element_seed: u64,
    course_index: i32,
    expose_top: bool,
    y0: i64,
    y1: i64,
    height_fraction: f64,
    terrain: &SiteTerrain,
    cut: &OpeningCut,
    intervals: &[(i64, i64)],
    d0: i64,
    d1: i64,
) {
    const STEPS: i64 = 4;
    let band_lo = cut.a_centre - cut.inner_half;
    let band_hi = cut.a_centre + cut.inner_half;
    if !intervals
        .iter()
        .any(|&(start, end)| start <= band_lo && end >= band_hi)
    {
        return;
    }
    let thickness = d1 - d0;
    if thickness <= 0 {
        return;
    }

    for step in 0..STEPS {
        let s0 = step as f64 / STEPS as f64;
        let s1 = (step + 1) as f64 / STEPS as f64;
        let d_a = d0 + round(thickness as f64 * s0) as i64;
        let d_b = d0 + round(thickness as f64 * s1) as i64;
        if d_b <= d_a {
            continue;
        }
        let half_a = cut.half_at(s0);
        let half_b = cut.half_at(s1);
        // Once the void has reached the inner width there is no solid left.
        if cut.inner_half - half_a < 10 && cut.inner_half - half_b < 10 {
            continue;
        }

        let front = step == 0;
        // The innermost step's back face collapses to a line because
        // `half_b == inner_half` at `s1 == 1`, so never emit it.
        let back = false;
        let top = expose_top;

        // Left jamb: run increases from the inner edge toward the void.
        let left = [
            (band_lo, d_a),
            (cut.a_centre - half_a, d_a),
            (cut.a_centre - half_b, d_b),
            (band_lo, d_b),
        ];
        push_jamb(
            out,
            wall,
            dir,
            right,
            element_seed,
            course_index,
            y0,
            y1,
            height_fraction,
            terrain,
            left,
            [false, top, front, back, false, true],
        );

        // Right jamb: mirror across the opening centreline.
        let right_plan = [
            (cut.a_centre + half_a, d_a),
            (band_hi, d_a),
            (band_hi, d_b),
            (cut.a_centre + half_b, d_b),
        ];
        push_jamb(
            out,
            wall,
            dir,
            right,
            element_seed,
            course_index,
            y0,
            y1,
            height_fraction,
            terrain,
            right_plan,
            [false, top, front, back, true, false],
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_jamb(
    out: &mut Vec<Block>,
    wall: &BatteredWall,
    dir: [f64; 2],
    right: [f64; 2],
    element_seed: u64,
    course_index: i32,
    y0: i64,
    y1: i64,
    height_fraction: f64,
    terrain: &SiteTerrain,
    plan: [(i64, i64); 4],
    expose: [bool; 6],
) {
    let point = |a: i64, d: i64, y: i64| -> [i64; 3] {
        let x = wall.start_mm[0] as f64 + dir[0] * a as f64 + right[0] * d as f64;
        let z = wall.start_mm[1] as f64 + dir[1] * a as f64 + right[1] * d as f64;
        [round(x) as i64, y, round(z) as i64]
    };
    let corners = [
        point(plan[0].0, plan[0].1, y0),
        point(plan[1].0, plan[1].1, y0),
        point(plan[2].0, plan[2].1, y0),
        point(plan[3].0, plan[3].1, y0),
        point(plan[0].0, plan[0].1, y1),
        point(plan[1].0, plan[1].1, y1),
        point(plan[2].0, plan[2].1, y1),
        point(plan[3].0, plan[3].1, y1),
    ];
    if is_buried(&corners, terrain) {
        return;
    }
    let da = (plan[1].0 - plan[0].0) as f64;
    let dd = (plan[1].1 - plan[0].1) as f64;
    let width = round(sqrt(da * da + dd * dd)) as i64;
    let run_pos = plan[0].0.min(plan[3].0);
    push_block(
        out,
        wall.id,
        ElementKind::BatteredWall,
        course_index,
        run_pos,
        corners,
        width.max(1),
        y1 - y0,
        element_seed,
        run_pos / 950,
        height_fraction,
        expose,
        BlockRole::Jamb,
    );
}

/// Shared block constructor for trimmed/heraldic stones.
#[allow(clippy::too_many_arguments)]
fn push_block(
    out: &mut Vec<Block>,
    element_id: u64,
    kind: ElementKind,
    course_index: i32,
    run_pos_mm: i64,
    corners: [[i64; 3]; 8],
    width_mm: i64,
    height_mm: i64,
    element_seed: u64,
    column: i64,
    height_fraction: f64,
    expose: [bool; 6],
    role: BlockRole,
) {
    out.push(Block {
        element_id,
        kind,
        course_index,
        run_pos_mm,
        build_index: 0,
        corners_mm: corners,
        block_width_mm: width_mm.max(1),
        block_height_mm: height_mm.max(1),
        tint: stone_tint(hash01(element_seed, course_index as i64, column, 11)),
        age: (0.55 + 0.45 * height_fraction as f32).min(1.0),
        jitter_seed: hash01(element_seed, course_index as i64, column, 12),
        expose,
        role,
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

/// Dress the stone terminating in a round tower so its end face follows the
/// tower's curve. For a forward seam the block's end corners (1, 2, 5, 6) are
/// moved independently; for a backward seam the start corners (0, 3, 4, 7) are.
#[allow(clippy::too_many_arguments)]
fn dress_tower_seam(
    out: &mut [Block],
    wall: &BatteredWall,
    dir: [f64; 2],
    right: [f64; 2],
    course_index: i32,
    d0: i64,
    d1: i64,
    y0: i64,
    y1: i64,
    seam: junction::TowerSeam,
) {
    let point = |a: i64, d: i64, y: i64| -> [i64; 3] {
        let x = wall.start_mm[0] as f64 + dir[0] * a as f64 + right[0] * d as f64;
        let z = wall.start_mm[1] as f64 + dir[1] * a as f64 + right[1] * d as f64;
        [round(x) as i64, y, round(z) as i64]
    };
    let corners = [
        point(seam.corners[0], d0, y0),
        point(seam.corners[1], d1, y0),
        point(seam.corners[2], d0, y1),
        point(seam.corners[3], d1, y1),
    ];
    let target = out.iter_mut().rev().find(|block| {
        if block.element_id != wall.id || block.course_index != course_index {
            return false;
        }
        if seam.forward {
            (block.run_pos_mm + block.block_width_mm - seam.nominal).abs() <= 1
        } else {
            (block.run_pos_mm - seam.nominal).abs() <= 1
        }
    });
    let Some(block) = target else {
        return;
    };
    if seam.forward {
        block.corners_mm[1] = corners[0];
        block.corners_mm[2] = corners[1];
        block.corners_mm[5] = corners[2];
        block.corners_mm[6] = corners[3];
        // The end face is buried in the tower shell; hiding it prevents a
        // coincident face from z-fighting inside the tower stones.
        block.expose[BlockFace::End1 as usize] = false;
    } else {
        block.corners_mm[0] = corners[0];
        block.corners_mm[3] = corners[1];
        block.corners_mm[4] = corners[2];
        block.corners_mm[7] = corners[3];
        block.expose[BlockFace::End0 as usize] = false;
    }
    block.role = BlockRole::SeamQuoin;
}

/// Dress the stone at a wall-to-wall corner so its end face lies along the
/// incumbent's battered face. The mitre corners are set independently, which
/// fills the triangular outer corner that a flat run cut leaves open.
#[allow(clippy::too_many_arguments)]
fn dress_wall_seam(
    out: &mut [Block],
    wall: &BatteredWall,
    dir: [f64; 2],
    right: [f64; 2],
    course_index: i32,
    d0: i64,
    d1: i64,
    y0: i64,
    y1: i64,
    seam: junction::WallSeam,
) {
    let point = |a: i64, d: i64, y: i64| -> [i64; 3] {
        let x = wall.start_mm[0] as f64 + dir[0] * a as f64 + right[0] * d as f64;
        let z = wall.start_mm[1] as f64 + dir[1] * a as f64 + right[1] * d as f64;
        [round(x) as i64, y, round(z) as i64]
    };
    let corners = [
        point(seam.corners[0], d0, y0),
        point(seam.corners[1], d1, y0),
        point(seam.corners[2], d0, y1),
        point(seam.corners[3], d1, y1),
    ];
    let target = if seam.forward {
        out.iter()
            .enumerate()
            .filter(|(_, b)| b.element_id == wall.id && b.course_index == course_index)
            .max_by_key(|(_, b)| b.run_pos_mm + b.block_width_mm)
            .map(|(index, _)| index)
    } else {
        out.iter()
            .enumerate()
            .filter(|(_, b)| b.element_id == wall.id && b.course_index == course_index)
            .min_by_key(|(_, b)| b.run_pos_mm)
            .map(|(index, _)| index)
    };
    let Some(index) = target else {
        return;
    };
    let block = &mut out[index];
    if seam.forward {
        block.corners_mm[1] = corners[0];
        block.corners_mm[2] = corners[1];
        block.corners_mm[5] = corners[2];
        block.corners_mm[6] = corners[3];
        block.expose[BlockFace::End1 as usize] = false;
    } else {
        block.corners_mm[0] = corners[0];
        block.corners_mm[3] = corners[1];
        block.corners_mm[4] = corners[2];
        block.corners_mm[7] = corners[3];
        block.expose[BlockFace::End0 as usize] = false;
    }
    block.role = BlockRole::SeamQuoin;
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
                role: BlockRole::Masonry,
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
    use crate::fixtures::{m0a_showcase_document, m0b_showcase_document};

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

    fn wall_frame(wall: &BatteredWall) -> ([f64; 2], [f64; 2]) {
        let dx = (wall.end_mm[0] - wall.start_mm[0]) as f64;
        let dz = (wall.end_mm[1] - wall.start_mm[1]) as f64;
        let len = sqrt(dx * dx + dz * dz);
        let dir = [dx / len, dz / len];
        (dir, [-dir[1], dir[0]])
    }

    fn local_point(wall: &BatteredWall, p: [i64; 3]) -> [f64; 3] {
        let (dir, right) = wall_frame(wall);
        let dx = p[0] as f64 - wall.start_mm[0] as f64;
        let dz = p[2] as f64 - wall.start_mm[1] as f64;
        [
            dx * dir[0] + dz * dir[1],
            dx * right[0] + dz * right[1],
            p[1] as f64,
        ]
    }

    #[test]
    fn wall_stones_stop_flush_at_the_battered_tower() {
        let terrain = SiteTerrain::m0a_promontory();
        let doc = m0b_showcase_document();
        let tower = match doc.get(3).unwrap() {
            Element::RoundTower(t) => t,
            _ => unreachable!(),
        };
        let wall = match doc.get(1).unwrap() {
            Element::BatteredWall(w) => w,
            _ => unreachable!(),
        };
        let blocks = generate_blocks(&doc, &terrain);
        let mut wall_stones = 0;
        let mut last_a = 0i64;
        let mut seam_corners = 0;
        for block in blocks.iter().filter(|b| b.element_id == 1) {
            wall_stones += 1;
            last_a = last_a.max(block.run_pos_mm + block.block_width_mm);
            for corner in block.corners_mm {
                let dx = corner[0] as f64 - tower.centre_mm[0] as f64;
                let dz = corner[2] as f64 - tower.centre_mm[1] as f64;
                let distance = sqrt(dx * dx + dz * dz);
                let inner = junction::tower_inner_radius_at(tower, &terrain, corner[1]) as f64;
                // No wall stone may ever reach the tower's inner face, even
                // though the seam is deliberately buried in the shell.
                assert!(
                    distance > inner,
                    "wall corner {corner:?} reached the tower's inner face"
                );
            }
            if block.role == BlockRole::SeamQuoin
                && block.run_pos_mm + block.block_width_mm > 20_000
            {
                for index in [1usize, 2, 5, 6] {
                    let corner = block.corners_mm[index];
                    let dx = corner[0] as f64 - tower.centre_mm[0] as f64;
                    let dz = corner[2] as f64 - tower.centre_mm[1] as f64;
                    let distance = sqrt(dx * dx + dz * dz);
                    let radius = junction::tower_outer_radius_at(tower, &terrain, corner[1]) as f64;
                    // Buried just enough to hide the flat tower facets, but
                    // never pushed through the shell.
                    assert!(
                        distance >= radius - (junction::TOWER_BURY_MM + 20) as f64,
                        "seam corner {corner:?} is {:.1} mm inside tower radius {radius:.1}",
                        radius - distance
                    );
                    assert!(
                        distance <= radius + 5.0,
                        "seam corner {corner:?} sits outside the tower surface"
                    );
                    seam_corners += 1;
                }
            }
        }
        assert!(wall_stones > 100);
        assert!(seam_corners > 0, "no dressed tower seam quoins");
        // The wall originally reached 18 m into the tower; after the solve it
        // must stop short of the tower's outer surface (a ~= 22.9 m of 26 m).
        assert!(
            last_a < 23_200,
            "wall still runs {last_a} mm along its 26000 mm length"
        );
        assert!(last_a > 22_000, "wall was over-trimmed to {last_a} mm");
        let _ = wall;
    }

    #[test]
    fn angled_wall_junction_has_no_overlap() {
        let terrain = SiteTerrain::m0a_promontory();
        let doc = m0b_showcase_document();
        let host = match doc.get(1).unwrap() {
            Element::BatteredWall(w) => w,
            _ => unreachable!(),
        };
        let (_, host_right) = wall_frame(host);
        let blocks = generate_blocks(&doc, &terrain);
        let mut checked = 0;
        for block in blocks.iter().filter(|b| b.element_id == 2) {
            for corner in block.corners_mm {
                let dx = corner[0] as f64 - host.start_mm[0] as f64;
                let dz = corner[2] as f64 - host.start_mm[1] as f64;
                let distance = (dx * host_right[0] + dz * host_right[1]).abs();
                // The lower wall is trimmed to the *narrowest* incumbent face
                // over its course so no daylight wedge can open as the batter
                // recedes. Near a plinth step that deliberately buries the
                // lower wall deeper, so compare against the narrowest face in
                // the course band rather than the face at the corner itself.
                let mut half = f64::MAX;
                let mut y = corner[1];
                while y <= corner[1] + 600 {
                    half = half.min(junction::wall_half_width_at(host, &terrain, y) as f64);
                    y += 50;
                }
                assert!(
                    distance >= half - (junction::SEAM_TUCK_MM + 40) as f64,
                    "wall 2 corner {corner:?} lies {:.1} mm inside wall 1's face",
                    half - distance
                );
                checked += 1;
            }
        }
        assert!(checked > 100);
    }

    fn point_in_polygon(point: [f64; 2], poly: &[[f64; 2]; 4]) -> bool {
        let mut inside = false;
        let mut j = 3;
        for i in 0..4 {
            let a = poly[i];
            let b = poly[j];
            if (a[1] > point[1]) != (b[1] > point[1]) {
                let x = (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0];
                if point[0] < x {
                    inside = !inside;
                }
            }
            j = i;
        }
        inside
    }

    /// Cross-section of a block at elevation `y`, interpolating between its
    /// bottom and top rings. Walls and towers are vertical prisms per course, so
    /// this is exact for any `y` inside the block.
    fn block_section(block: &Block, y: f64) -> [[f64; 2]; 4] {
        let y0 = block.corners_mm[0][1] as f64;
        let y1 = block.corners_mm[4][1] as f64;
        let t = if (y1 - y0).abs() < 1.0 {
            0.0
        } else {
            ((y - y0) / (y1 - y0)).clamp(0.0, 1.0)
        };
        let mut poly = [[0.0f64; 2]; 4];
        for i in 0..4 {
            let b = block.corners_mm[i];
            let u = block.corners_mm[i + 4];
            poly[i] = [
                b[0] as f64 + (u[0] - b[0]) as f64 * t,
                b[2] as f64 + (u[2] - b[2]) as f64 * t,
            ];
        }
        poly
    }

    /// True when any block covers `(x, z)` at elevation `y`. A sub-millimetre
    /// nudge keeps points on shared joints from falling between two stones that
    /// meet there exactly.
    fn covered(blocks: &[Block], x: f64, z: f64, y: f64) -> bool {
        for ox in [-0.75f64, 0.0, 0.75] {
            for oz in [-0.75f64, 0.0, 0.75] {
                let point = [x + ox, z + oz];
                for block in blocks {
                    let lo = block.corners_mm[0][1].min(block.corners_mm[4][1]) as f64 - 1.0;
                    let hi = block.corners_mm[0][1].max(block.corners_mm[4][1]) as f64 + 1.0;
                    if y < lo || y > hi {
                        continue;
                    }
                    if point_in_polygon(point, &block_section(block, y)) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn sampled_elevations(low: i64, high: i64) -> [i64; 3] {
        let low = low.max(1);
        let high = high.max(low + 3);
        [low, (low + high) / 2, high]
    }

    #[test]
    fn tower_seam_is_watertight_at_low_mid_high() {
        let terrain = SiteTerrain::m0a_promontory();
        let doc = m0b_showcase_document();
        let tower = match doc.get(3).unwrap() {
            Element::RoundTower(t) => t,
            _ => unreachable!(),
        };
        let wall = match doc.get(1).unwrap() {
            Element::BatteredWall(w) => w,
            _ => unreachable!(),
        };
        let blocks = generate_blocks(&doc, &terrain);
        let (_, wall_plinth, wall_top) = junction::wall_elevations(wall, &terrain);
        let (_, tower_plinth, tower_top) = junction::tower_elevations(tower, &terrain);
        let low = wall_plinth.max(tower_plinth) + 1_200;
        let high = wall_top.min(tower_top) - 1_200;
        let (dir, right) = wall_frame(wall);
        let start = [wall.start_mm[0] as f64, wall.start_mm[1] as f64];
        let centre = [tower.centre_mm[0] as f64, tower.centre_mm[1] as f64];
        let mut samples = 0;

        for y in sampled_elevations(low, high) {
            let yf = y as f64;
            let radius = junction::tower_outer_radius_at(tower, &terrain, y) as f64;
            let half = junction::wall_half_width_at(wall, &terrain, y) as f64;
            // Angular span of the wall's cross-section at the tower surface.
            let mut angles = [0.0f64; 2];
            for (index, d) in [-half, half].into_iter().enumerate() {
                let q = [
                    start[0] + right[0] * d - centre[0],
                    start[1] + right[1] * d - centre[1],
                ];
                let b = q[0] * dir[0] + q[1] * dir[1];
                let c = q[0] * q[0] + q[1] * q[1] - radius * radius;
                let root = sqrt((b * b - c).max(0.0));
                let a = -b - root;
                let px = start[0] + dir[0] * a + right[0] * d;
                let pz = start[1] + dir[1] * a + right[1] * d;
                angles[index] = libm::atan2(pz - centre[1], px - centre[0]);
            }
            let mut a0 = angles[0];
            let mut a1 = angles[1];
            if a1 - a0 > std::f64::consts::PI {
                a0 += std::f64::consts::TAU;
            } else if a0 - a1 > std::f64::consts::PI {
                a1 += std::f64::consts::TAU;
            }
            // Sample just inside the facet radius, where a flat tower facet sits
            // up to ~33 mm inside the true circle. Only the wall can cover those
            // points if the seam is open.
            let sample_radius = radius - 10.0;
            for step in 1..20 {
                let t = step as f64 / 20.0;
                let angle = a0 + (a1 - a0) * t;
                let x = centre[0] + cos(angle) * sample_radius;
                let z = centre[1] + sin(angle) * sample_radius;
                assert!(
                    covered(&blocks, x, z, yf),
                    "daylight at tower seam y={y} angle={angle:.3} point=({x:.0},{z:.0})"
                );
                samples += 1;
            }
        }
        assert!(samples >= 30);
    }

    #[test]
    fn angled_wall_corner_is_watertight_at_low_mid_high() {
        let terrain = SiteTerrain::m0a_promontory();
        let doc = m0b_showcase_document();
        let host = match doc.get(1).unwrap() {
            Element::BatteredWall(w) => w,
            _ => unreachable!(),
        };
        let ret = match doc.get(2).unwrap() {
            Element::BatteredWall(w) => w,
            _ => unreachable!(),
        };
        let blocks = generate_blocks(&doc, &terrain);
        let (_, host_plinth, host_top) = junction::wall_elevations(host, &terrain);
        let (_, ret_plinth, ret_top) = junction::wall_elevations(ret, &terrain);
        let low = host_plinth.max(ret_plinth) + 1_000;
        let high = host_top.min(ret_top) - 1_000;
        let (host_dir, host_right) = wall_frame(host);
        let (ret_dir, ret_right) = wall_frame(ret);
        let corner = [host.start_mm[0] as f64, host.start_mm[1] as f64];
        let mut samples = 0;

        for y in sampled_elevations(low, high) {
            let yf = y as f64;
            let (hext_s, hext_e) = junction::wall_corner_extensions(&doc, host, y, y, &terrain);
            let (rext_s, rext_e) = junction::wall_corner_extensions(&doc, ret, y, y, &terrain);
            let host_half = junction::wall_half_width_at(host, &terrain, y) as f64;
            let ret_half = junction::wall_half_width_at(ret, &terrain, y) as f64;
            let host_len = host.length_mm() as f64;
            let ret_len = ret.length_mm() as f64;
            // Sample a little inside each slab so boundary quantisation does
            // not masquerade as daylight.
            let inset = 120.0;
            let span = (host_half.max(ret_half) + 1_500.0).max(3_500.0);
            let mut x = corner[0] - span;
            while x <= corner[0] + span {
                let mut z = corner[1] - span;
                while z <= corner[1] + span {
                    let dx = x - host.start_mm[0] as f64;
                    let dz = z - host.start_mm[1] as f64;
                    let host_d = (dx * host_right[0] + dz * host_right[1]).abs();
                    let host_a = dx * host_dir[0] + dz * host_dir[1];
                    let in_host = host_d <= host_half - inset
                        && host_a >= -hext_s as f64 + inset
                        && host_a <= host_len + hext_e as f64 - inset;
                    let rdx = x - ret.start_mm[0] as f64;
                    let rdz = z - ret.start_mm[1] as f64;
                    let ret_d = (rdx * ret_right[0] + rdz * ret_right[1]).abs();
                    let ret_a = rdx * ret_dir[0] + rdz * ret_dir[1];
                    let in_ret = ret_d <= ret_half - inset
                        && ret_a >= -rext_s as f64 + inset
                        && ret_a <= ret_len + rext_e as f64 - inset;
                    if in_host || in_ret {
                        assert!(
                            covered(&blocks, x, z, yf),
                            "daylight in angled corner y={y} point=({x:.0},{z:.0})"
                        );
                        samples += 1;
                    }
                    z += 200.0;
                }
                x += 200.0;
            }
        }
        assert!(
            samples > 200,
            "corner sampling only covered {samples} points"
        );
    }

    #[test]
    fn overlapping_openings_are_not_carved() {
        let terrain = SiteTerrain::m0a_promontory();
        let mut doc = CastleDocument::new();
        doc.add_wall(BatteredWall::new(
            1,
            "wall",
            [0, 0],
            [26_000, 0],
            9_000,
            2_500,
        ));
        doc.add_opening(Opening::arrow_loop(10, 1, "a", 300, 1_800));
        doc.add_opening(Opening::arrow_loop(11, 1, "b", 320, 1_800));
        let blocks = generate_blocks(&doc, &terrain);
        let wall_blocks: Vec<&Block> = blocks.iter().filter(|b| b.element_id == 1).collect();
        assert!(
            !wall_blocks.iter().any(|b| matches!(
                b.role,
                BlockRole::Jamb | BlockRole::Sill | BlockRole::Lintel
            )),
            "overlapping openings must not leave a partially framed hole"
        );
    }

    #[test]
    fn opening_crossing_a_junction_seam_is_not_carved() {
        let terrain = SiteTerrain::m0a_promontory();
        let mut doc = CastleDocument::new();
        doc.add_wall(BatteredWall::new(
            1,
            "wall",
            [0, 0],
            [10_000, 0],
            9_000,
            2_500,
        ));
        doc.add_tower(RoundTower::new(2, "tower", [10_000, 0], 2_000, 12_000));
        // The inner splay band would run into the tower-trimmed end.
        doc.add_opening(Opening::arrow_loop(10, 1, "seam loop", 950, 1_800));
        let blocks = generate_blocks(&doc, &terrain);
        let wall_blocks: Vec<&Block> = blocks.iter().filter(|b| b.element_id == 1).collect();
        assert!(
            !wall_blocks.iter().any(|b| matches!(
                b.role,
                BlockRole::Jamb | BlockRole::Sill | BlockRole::Lintel
            )),
            "an opening that does not fit inside the trimmed run must not be carved"
        );
    }

    #[test]
    fn arrow_loop_void_is_open_and_framed() {
        let terrain = SiteTerrain::m0a_promontory();
        let doc = m0b_showcase_document();
        let wall = match doc.get(1).unwrap() {
            Element::BatteredWall(w) => w,
            _ => unreachable!(),
        };
        let opening = doc.opening(10).unwrap();
        let cut = OpeningCut::resolve(wall, opening, &terrain);
        assert!(cut.inner_half > cut.outer_half);

        let blocks = generate_blocks(&doc, &terrain);
        let wall_blocks: Vec<&Block> = blocks.iter().filter(|b| b.element_id == 1).collect();

        // Sill, lintel and jamb stones must exist around the void.
        assert!(wall_blocks.iter().any(|b| b.role == BlockRole::Jamb));
        assert!(wall_blocks.iter().any(|b| b.role == BlockRole::Sill));
        assert!(wall_blocks.iter().any(|b| b.role == BlockRole::Lintel));

        // The central slit path must be free of masonry at every depth.
        let mut samples = 0;
        let mut y = cut.sill_y + 150;
        while y < cut.lintel_y - 150 {
            let mut d = -wall.thickness_mm / 2 + 600;
            while d <= wall.thickness_mm / 2 - 600 {
                let plan = opening.plan_point_mm(wall);
                let (dir, right) = wall_frame(wall);
                let x = plan[0] as f64 + right[0] * d as f64;
                let z = plan[1] as f64 + right[1] * d as f64;
                let point = [round(x) as i64, y, round(z) as i64];
                for block in &wall_blocks {
                    let mut min = [f64::MAX; 3];
                    let mut max = [f64::MIN; 3];
                    for corner in block.corners_mm {
                        let local = local_point(wall, corner);
                        for axis in 0..3 {
                            min[axis] = min[axis].min(local[axis]);
                            max[axis] = max[axis].max(local[axis]);
                        }
                    }
                    let local = local_point(wall, point);
                    let inside = (0..3)
                        .all(|axis| local[axis] > min[axis] + 0.5 && local[axis] < max[axis] - 0.5);
                    assert!(
                        !inside,
                        "block {:?} role {:?} occludes the arrow loop at {point:?}",
                        block.run_pos_mm, block.role
                    );
                }
                let _ = dir;
                samples += 1;
                d += 200;
            }
            y += 200;
        }
        assert!(samples > 10);
    }
}
