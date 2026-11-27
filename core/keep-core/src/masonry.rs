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
    BatteredWall, CastleDocument, Element, ElementKind, Gatehouse, Keep, Opening, Range, RangeKind,
    RoundTower, TowerPlan, CASTLE_SEED,
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
    /// A coursed roof tile, slate or lead strip on a pitched range roof.
    Roof,
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
            Element::Gatehouse(gatehouse) => layout_gatehouse(gatehouse, terrain, &mut blocks),
            Element::Keep(keep) => layout_keep(keep, terrain, &mut blocks),
            Element::Range(range) => layout_range(range, document, terrain, &mut blocks),
            Element::Ditch(_) => {}
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
        let is_plinth = y0 < plinth_top_y;
        let plinth = if is_plinth {
            wall.plinth_projection_mm
        } else {
            0
        };
        // Every course carries a top cap, not just the final course or the
        // plinth ledge. Construction playback hides the course above while the
        // active course rises, so exposing only the final cap would show a
        // hollow, paper-thin shell mid-build.
        let expose_top = true;

        // True sloped batter: evaluate the face at the course bed and its top
        // separately so stacked courses share an exact horizontal plane. The
        // plinth projection stays constant across the course and steps cleanly
        // at the capped plinth ledge.
        let (d0_bot, d1_bot) = wall_face_offsets(wall, foundation_y, total_height, y0, plinth);
        let (d0_top, d1_top) = wall_face_offsets(wall, foundation_y, total_height, y1, plinth);

        // Priority-ordered junction trimming: wall stones may only exist in
        // these run intervals once towers and higher-priority walls have taken
        // their volume. The higher-priority wall of an angled corner is
        // extended past the shared endpoint so the outer mitre stays solid.
        let (extend_start, extend_end) =
            junction::wall_corner_extensions(document, wall, y0, y1, terrain);
        let run_lo = -extend_start;
        let run_hi = length_mm + extend_end;
        let intervals = junction::wall_course_intervals(
            document, wall, d0_bot, d1_bot, d0_top, d1_top, run_lo, run_hi, y0, y1, terrain,
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
                d0_bot,
                d1_bot,
                d0_top,
                d1_top,
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
                    d0_bot,
                    d1_bot,
                    d0_top,
                    d1_top,
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
                    d0_bot,
                    d1_bot,
                    d0_top,
                    d1_top,
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
            if let Some(seam) = junction::tower_seam(
                document, wall, d0_bot, d1_bot, d0_top, d1_top, y0, y1, terrain,
            ) {
                dress_tower_seam(
                    out,
                    wall,
                    dir,
                    right,
                    course_index,
                    d0_bot,
                    d1_bot,
                    d0_top,
                    d1_top,
                    y0,
                    y1,
                    seam,
                );
            }
            // ...and the stone that meets a higher-priority wall so its end
            // face lies along the incumbent's battered face instead of a flat
            // run cut that leaves a triangular daylight gap.
            for seam in junction::wall_seams(
                document, wall, d0_bot, d1_bot, d0_top, d1_top, y0, y1, terrain,
            ) {
                dress_wall_seam(
                    out,
                    wall,
                    dir,
                    right,
                    course_index,
                    d0_bot,
                    d1_bot,
                    d0_top,
                    d1_top,
                    y0,
                    y1,
                    seam,
                );
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

/// Whole-course layout for runs that need no junction trimming or openings.
/// Joints are still solved once per joint so neighbouring stones share an
/// exact plane.
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
    d0_bot: i64,
    d1_bot: i64,
    d0_top: i64,
    d1_top: i64,
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
                    d0_bot,
                    d1_bot,
                    d0_top,
                    d1_top,
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
            d0_bot,
            d1_bot,
            d0_top,
            d1_top,
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
    d0_bot: i64,
    d1_bot: i64,
    d0_top: i64,
    d1_top: i64,
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
                    d0_bot,
                    d1_bot,
                    d0_top,
                    d1_top,
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
            d0_bot,
            d1_bot,
            d0_top,
            d1_top,
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
    d0_bot: i64,
    d1_bot: i64,
    d0_top: i64,
    d1_top: i64,
    y0: i64,
    y1: i64,
    height_fraction: f64,
    terrain: &SiteTerrain,
    expose_start: bool,
    expose_end: bool,
    role: BlockRole,
) {
    let column = a0 / 950;
    let corners = wall_corners(
        wall, dir, right, a0, a1, d0_bot, d1_bot, d0_top, d1_top, y0, y1,
    );
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

/// Whole-course block emitter for an untrimmed run.
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
    d0_bot: i64,
    d1_bot: i64,
    d0_top: i64,
    d1_top: i64,
    y0: i64,
    y1: i64,
    height_fraction: f64,
    terrain: &SiteTerrain,
) {
    let column = a0 / 950;

    let corners = wall_corners(
        wall, dir, right, a0, a1, d0_bot, d1_bot, d0_top, d1_top, y0, y1,
    );

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
    d0_bot: i64,
    d1_bot: i64,
    d0_top: i64,
    d1_top: i64,
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
    let thickness_bot = d1_bot - d0_bot;
    let thickness_top = d1_top - d0_top;
    if thickness_bot <= 0 || thickness_top <= 0 {
        return;
    }

    for step in 0..STEPS {
        let s0 = step as f64 / STEPS as f64;
        let s1 = (step + 1) as f64 / STEPS as f64;
        let d_a_bot = d0_bot + round(thickness_bot as f64 * s0) as i64;
        let d_b_bot = d0_bot + round(thickness_bot as f64 * s1) as i64;
        let d_a_top = d0_top + round(thickness_top as f64 * s0) as i64;
        let d_b_top = d0_top + round(thickness_top as f64 * s1) as i64;
        if d_b_bot <= d_a_bot || d_b_top <= d_a_top {
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
        let left_bot = [
            (band_lo, d_a_bot),
            (cut.a_centre - half_a, d_a_bot),
            (cut.a_centre - half_b, d_b_bot),
            (band_lo, d_b_bot),
        ];
        let left_top = [
            (band_lo, d_a_top),
            (cut.a_centre - half_a, d_a_top),
            (cut.a_centre - half_b, d_b_top),
            (band_lo, d_b_top),
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
            left_bot,
            left_top,
            [false, top, front, back, false, true],
        );

        // Right jamb: mirror across the opening centreline.
        let right_bot = [
            (cut.a_centre + half_a, d_a_bot),
            (band_hi, d_a_bot),
            (band_hi, d_b_bot),
            (cut.a_centre + half_b, d_b_bot),
        ];
        let right_top = [
            (cut.a_centre + half_a, d_a_top),
            (band_hi, d_a_top),
            (band_hi, d_b_top),
            (cut.a_centre + half_b, d_b_top),
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
            right_bot,
            right_top,
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
    plan_bot: [(i64, i64); 4],
    plan_top: [(i64, i64); 4],
    expose: [bool; 6],
) {
    let point = |a: i64, d: i64, y: i64| -> [i64; 3] {
        let x = wall.start_mm[0] as f64 + dir[0] * a as f64 + right[0] * d as f64;
        let z = wall.start_mm[1] as f64 + dir[1] * a as f64 + right[1] * d as f64;
        [round(x) as i64, y, round(z) as i64]
    };
    let corners = [
        point(plan_bot[0].0, plan_bot[0].1, y0),
        point(plan_bot[1].0, plan_bot[1].1, y0),
        point(plan_bot[2].0, plan_bot[2].1, y0),
        point(plan_bot[3].0, plan_bot[3].1, y0),
        point(plan_top[0].0, plan_top[0].1, y1),
        point(plan_top[1].0, plan_top[1].1, y1),
        point(plan_top[2].0, plan_top[2].1, y1),
        point(plan_top[3].0, plan_top[3].1, y1),
    ];
    if is_buried(&corners, terrain) {
        return;
    }
    let da = (plan_bot[1].0 - plan_bot[0].0) as f64;
    let dd = (plan_bot[1].1 - plan_bot[0].1) as f64;
    let width = round(sqrt(da * da + dd * dd)) as i64;
    let run_pos = plan_bot[0].0.min(plan_bot[3].0);
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
    d0_bot: i64,
    d1_bot: i64,
    d0_top: i64,
    d1_top: i64,
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
        point(seam.corners[0], d0_bot, y0),
        point(seam.corners[1], d1_bot, y0),
        point(seam.corners[2], d0_top, y1),
        point(seam.corners[3], d1_top, y1),
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
    d0_bot: i64,
    d1_bot: i64,
    d0_top: i64,
    d1_top: i64,
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
        point(seam.corners[0], d0_bot, y0),
        point(seam.corners[1], d1_bot, y0),
        point(seam.corners[2], d0_top, y1),
        point(seam.corners[3], d1_top, y1),
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
    d0_bot: i64,
    d1_bot: i64,
    d0_top: i64,
    d1_top: i64,
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
        point(a0, d0_bot, y0),
        point(a1, d0_bot, y0),
        point(a1, d1_bot, y0),
        point(a0, d1_bot, y0),
        point(a0, d0_top, y1),
        point(a1, d0_top, y1),
        point(a1, d1_top, y1),
        point(a0, d1_top, y1),
    ]
}

fn layout_tower(tower: &RoundTower, terrain: &SiteTerrain, out: &mut Vec<Block>) {
    match tower.plan {
        TowerPlan::Round => layout_round_tower(tower, terrain, out),
        TowerPlan::Square => layout_shaped_tower(tower, terrain, out, TowerPlan::Square),
        TowerPlan::DShaped => layout_shaped_tower(tower, terrain, out, TowerPlan::DShaped),
    }
}

fn layout_round_tower(tower: &RoundTower, terrain: &SiteTerrain, out: &mut Vec<Block>) {
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
        let is_plinth = y0 < plinth_top_y;
        let plinth = if is_plinth {
            tower.plinth_projection_mm
        } else {
            0
        };
        // Every course is capped; see `layout_wall` for why mid-build courses
        // must expose their top face.
        let expose_top = true;
        // Sloped batter: bed and top rings get their own radii so stacked
        // courses share an exact horizontal plane. The plinth projection stays
        // constant within the course and steps at the capped ledge.
        let (r_out_bot, r_in_bot) =
            tower_face_radii(tower, thickness, foundation_y, total_height, y0, plinth);
        let (r_out_top, r_in_top) =
            tower_face_radii(tower, thickness, foundation_y, total_height, y1, plinth);

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

            let corners = tower_corners(
                tower.centre_mm,
                a0,
                a1,
                r_out_bot,
                r_in_bot,
                r_out_top,
                r_in_top,
                y0,
                y1,
            );
            if is_buried(&corners, terrain) {
                continue;
            }

            let width = round(r_out_bot as f64 * step) as i64;
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

/// Square and D-shaped towers share the round tower's course/batter machinery.
/// Square plans are generated as a four-sided ring whose boundary point is the
/// Chebyshev radius, so adjacent stones share exact vertical joints and the
/// corner blocks meet on the diagonal. D-shaped plans generate a curved front
/// drum plus a straight rear gorge wall.
fn layout_shaped_tower(
    tower: &RoundTower,
    terrain: &SiteTerrain,
    out: &mut Vec<Block>,
    plan: TowerPlan,
) {
    let radius = tower.radius_mm.max(500);
    let thickness = tower.thickness_mm.clamp(300, radius);
    let (ground_min, ground_max) = ground_range_tower(tower, terrain);
    let foundation_y = ground_min - 1_200;
    let plinth_top_y = ground_max + tower.plinth_height_mm;
    let top_y = ground_max + tower.height_mm;
    let total_height = (top_y - foundation_y).max(1);
    let element_seed = tower.seed ^ CASTLE_SEED.rotate_left(13);
    let orientation = tower.orientation_deg;

    let arc_count = 14i32;
    let arc_step = std::f64::consts::PI / arc_count as f64;

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
        let is_plinth = y0 < plinth_top_y;
        let plinth = if is_plinth {
            tower.plinth_projection_mm
        } else {
            0
        };
        // Every course is capped; see `layout_wall` for why mid-build courses
        // must expose their top face.
        let expose_top = true;
        let (r_out_bot, r_in_bot) =
            tower_face_radii(tower, thickness, foundation_y, total_height, y0, plinth);
        let (r_out_top, r_in_top) =
            tower_face_radii(tower, thickness, foundation_y, total_height, y1, plinth);

        match plan {
            TowerPlan::Square => {
                let bond = if course_index % 2 == 0 { 0 } else { 475 };
                // Alternate which pair of faces owns the corners. The owning
                // faces run the full battered square width; the other pair stops
                // at the inner face, so the two never overlap and every corner
                // stays a crisp 90 degrees on every course.
                let u_faces_own = course_index % 2 == 0;
                for face in [
                    SquareFace::PosU,
                    SquareFace::NegU,
                    SquareFace::PosV,
                    SquareFace::NegV,
                ] {
                    let owns_corner = match face {
                        SquareFace::PosU | SquareFace::NegU => u_faces_own,
                        SquareFace::PosV | SquareFace::NegV => !u_faces_own,
                    };
                    let (a0_bot, a1_bot) = if owns_corner {
                        (-r_out_bot, r_out_bot)
                    } else {
                        (-r_in_bot, r_in_bot)
                    };
                    let (a0_top, a1_top) = if owns_corner {
                        (-r_out_top, r_out_top)
                    } else {
                        (-r_in_top, r_in_top)
                    };
                    let (d0_bot, d1_bot) = match face {
                        SquareFace::PosU | SquareFace::NegV => (-r_out_bot, -r_in_bot),
                        SquareFace::NegU | SquareFace::PosV => (r_in_bot, r_out_bot),
                    };
                    let (d0_top, d1_top) = match face {
                        SquareFace::PosU | SquareFace::NegV => (-r_out_top, -r_in_top),
                        SquareFace::NegU | SquareFace::PosV => (r_in_top, r_out_top),
                    };
                    let face_index = face.index();
                    let joints = joint_positions(
                        a0_bot,
                        a1_bot,
                        950,
                        bond,
                        element_seed,
                        course_index,
                        face_index * 1_000,
                    );
                    let count = joints.len() - 1;
                    for (segment, pair) in joints.windows(2).enumerate() {
                        let (a0b, a1b) = (pair[0], pair[1]);
                        let a0t = map_run(a0b, a0_bot, a1_bot, a0_top, a1_top);
                        let a1t = map_run(a1b, a0_bot, a1_bot, a0_top, a1_top);
                        if a1b <= a0b || a1t <= a0t {
                            continue;
                        }
                        let corners = square_face_corners(
                            tower.centre_mm,
                            orientation,
                            face,
                            a0b,
                            a1b,
                            a0t,
                            a1t,
                            d0_bot,
                            d1_bot,
                            d0_top,
                            d1_top,
                            y0,
                            y1,
                        );
                        if is_buried(&corners, terrain) {
                            continue;
                        }
                        let column = face_index * 100_000 + segment as i32;
                        out.push(Block {
                            element_id: tower.id,
                            kind: ElementKind::RoundTower,
                            course_index,
                            run_pos_mm: column as i64,
                            build_index: 0,
                            corners_mm: corners,
                            block_width_mm: (a1b - a0b).max(1),
                            block_height_mm: (y1 - y0).max(1),
                            tint: stone_tint(hash01(
                                element_seed,
                                course_index as i64,
                                column as i64,
                                11,
                            )),
                            age: (0.55 + 0.45 * t as f32).min(1.0),
                            jitter_seed: hash01(
                                element_seed,
                                course_index as i64,
                                column as i64,
                                12,
                            ),
                            expose: [
                                false,
                                expose_top,
                                true,
                                true,
                                owns_corner && segment == 0,
                                owns_corner && segment + 1 == count,
                            ],
                            role: BlockRole::Masonry,
                        });
                    }
                }
            }
            TowerPlan::DShaped => {
                // Half-step running bond on alternating courses, shared joint
                // jitter, with both ends pinned at the flat gorge wall so the
                // arc still meets it exactly.
                let bond = if course_index % 2 == 0 {
                    0.0
                } else {
                    arc_step * 0.5
                };
                let joint_angle = |joint: i32| -> f64 {
                    if joint <= 0 {
                        -std::f64::consts::FRAC_PI_2
                    } else if joint >= arc_count {
                        std::f64::consts::FRAC_PI_2
                    } else {
                        let jitter_mm = stone_jitter(element_seed, course_index, joint, 3) as f64;
                        -std::f64::consts::FRAC_PI_2
                            + arc_step * joint as f64
                            + bond
                            + jitter_mm / radius as f64
                    }
                };
                for j in 0..arc_count {
                    let a0 = joint_angle(j);
                    let a1 = joint_angle(j + 1);
                    if a1 <= a0 {
                        continue;
                    }
                    let corners = dshaped_arc_corners(
                        tower.centre_mm,
                        a0,
                        a1,
                        r_out_bot,
                        r_in_bot,
                        r_out_top,
                        r_in_top,
                        orientation,
                        y0,
                        y1,
                    );
                    if is_buried(&corners, terrain) {
                        continue;
                    }
                    let width = round(r_out_bot as f64 * (a1 - a0)) as i64;
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
                // Straight rear gorge wall on the flat back of the D. The curved
                // arc occupies `u >= 0`; the rear wall occupies `u in [-thickness, 0]`
                // across the full outer width `[-r_out, +r_out]` (tapered from
                // `r_out_bot` to `r_out_top`), closing flush against both ends of
                // the arc with zero gap and zero overlapping volume.
                let bond = if course_index % 2 == 0 { 0 } else { 475 };
                if r_out_bot > 0 {
                    let joints = joint_positions(
                        -r_out_bot,
                        r_out_bot,
                        950,
                        bond,
                        element_seed,
                        course_index,
                        50_000,
                    );
                    let col_count = joints.len().saturating_sub(1);
                    for (column, pair) in joints.windows(2).enumerate() {
                        let (v0_bot, v1_bot) = (pair[0], pair[1]);
                        let v0_top = map_radius(v0_bot, r_out_bot, r_out_top);
                        let v1_top = map_radius(v1_bot, r_out_bot, r_out_top);
                        if v1_bot <= v0_bot || v1_top <= v0_top {
                            continue;
                        }
                        let corners = dshaped_rear_corners(
                            tower.centre_mm,
                            orientation,
                            v0_bot,
                            v1_bot,
                            v0_top,
                            v1_top,
                            -1,
                            -thickness,
                            y0,
                            y1,
                        );
                        if !is_buried(&corners, terrain) {
                            let expose_end0 = column == 0;
                            let expose_end1 = column + 1 == col_count;
                            out.push(Block {
                                element_id: tower.id,
                                kind: ElementKind::RoundTower,
                                course_index,
                                run_pos_mm: 2_000_000 + v0_bot,
                                build_index: 0,
                                corners_mm: corners,
                                block_width_mm: (v1_bot - v0_bot).max(1),
                                block_height_mm: (y1 - y0).max(1),
                                tint: stone_tint(hash01(
                                    element_seed,
                                    course_index as i64,
                                    10_000 + column as i64,
                                    11,
                                )),
                                age: (0.55 + 0.45 * t as f32).min(1.0),
                                jitter_seed: hash01(
                                    element_seed,
                                    course_index as i64,
                                    10_000 + column as i64,
                                    12,
                                ),
                                expose: [false, expose_top, true, true, expose_end0, expose_end1],
                                role: BlockRole::Masonry,
                            });
                        }
                    }
                }
            }
            TowerPlan::Round => {}
        }

        course_index += 1;
        y0 = y1;
    }
}

// ---------------------------------------------------------------------------
// Keeps and roofed ranges (M2.1)
// ---------------------------------------------------------------------------

/// Wall thickness used by a domestic range, derived from its height.
fn range_wall_thickness_mm(range: &Range) -> i64 {
    (range.wall_height_mm / 8).clamp(600, 1_200)
}

/// Ground height range sampled over an oriented rectangle footprint.
fn ground_range_rect(
    centre_mm: [i64; 2],
    width_mm: i64,
    depth_mm: i64,
    orientation_deg: i32,
    terrain: &SiteTerrain,
) -> (i64, i64) {
    let half_u = width_mm.max(1) as f64 * 0.5;
    let half_v = depth_mm.max(1) as f64 * 0.5;
    let mut min_h = i64::MAX;
    let mut max_h = i64::MIN;
    for iu in -2..=2 {
        for iv in -2..=2 {
            let u = half_u * iu as f64 / 2.0;
            let v = half_v * iv as f64 / 2.0;
            let (x, z) = local_to_world(centre_mm, orientation_deg, u, v);
            let h = terrain.height_mm(round(x) as i64, round(z) as i64);
            min_h = min_h.min(h);
            max_h = max_h.max(h);
        }
    }
    (min_h, max_h)
}

fn local_to_world(centre_mm: [i64; 2], orientation_deg: i32, u: f64, v: f64) -> (f64, f64) {
    let o = orientation_deg as f64 * std::f64::consts::PI / 180.0;
    let (oc, os) = (cos(o), sin(o));
    (
        centre_mm[0] as f64 + u * oc - v * os,
        centre_mm[1] as f64 + u * os + v * oc,
    )
}

/// Eight corners of an oriented box. Corner order follows the shared
/// [`Block`] contract: `0->1` runs along `u`, `0->3` along `v`, and `4..7`
/// repeat the bottom ring one thickness higher.
#[allow(clippy::too_many_arguments)]
fn oriented_box_corners(
    centre_mm: [i64; 2],
    orientation_deg: i32,
    u0: i64,
    u1: i64,
    v0: i64,
    v1: i64,
    y0: i64,
    y1: i64,
) -> [[i64; 3]; 8] {
    let p = |u: i64, v: i64, y: i64| -> [i64; 3] {
        let (x, z) = local_to_world(centre_mm, orientation_deg, u as f64, v as f64);
        [round(x) as i64, y, round(z) as i64]
    };
    [
        p(u0, v0, y0),
        p(u1, v0, y0),
        p(u1, v1, y0),
        p(u0, v1, y0),
        p(u0, v0, y1),
        p(u1, v0, y1),
        p(u1, v1, y1),
        p(u0, v1, y1),
    ]
}

#[allow(clippy::too_many_arguments)]
fn push_custom_block(
    out: &mut Vec<Block>,
    element_id: u64,
    kind: ElementKind,
    course_index: i32,
    run_pos_mm: i64,
    corners: [[i64; 3]; 8],
    width_mm: i64,
    height_mm: i64,
    seed: u64,
    tint: f32,
    age: f32,
    role: BlockRole,
    expose: [bool; 6],
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
        tint,
        age,
        jitter_seed: hash01(seed, course_index as i64, run_pos_mm, 200 + role as i64),
        expose,
        role,
    });
}

/// Lay out a great rectangular keep: four battered walls that alternate which
/// pair owns the corners, so every course meets in crisp 90-degree quoins, and
/// a crenellated parapet of merlons along the top.
fn layout_keep(keep: &Keep, terrain: &SiteTerrain, out: &mut Vec<Block>) {
    let half_w = (keep.width_mm.max(4_000) / 2).max(2_000);
    let half_d = (keep.depth_mm.max(4_000) / 2).max(2_000);
    let thickness = keep
        .wall_thickness_mm
        .clamp(400, half_w.min(half_d).saturating_sub(200));
    let (ground_min, ground_max) = ground_range_rect(
        keep.centre_mm,
        keep.width_mm,
        keep.depth_mm,
        keep.orientation_deg,
        terrain,
    );
    let foundation_y = ground_min - 1_200;
    let plinth_top_y = ground_max + keep.plinth_height_mm.max(0);
    let top_y = ground_max + keep.height_mm.max(1);
    let total_height = (top_y - foundation_y).max(1);
    let element_seed = keep.seed ^ CASTLE_SEED.rotate_left(17);

    let course_nominal = 430i64;
    let mut course_index = 0i32;
    let mut y0 = foundation_y;

    while y0 < top_y {
        let jitter = (hash01(element_seed, i64::from(course_index), 2, 0) - 0.5) * 30.0;
        let mut y1 = y0 + course_nominal + jitter as i64;
        if y1 > top_y || top_y - y1 < course_nominal / 2 {
            y1 = top_y;
        }
        if y1 <= y0 {
            break;
        }

        let mid_y = (y0 + y1) / 2;
        let t = ((mid_y - foundation_y) as f64 / total_height as f64).clamp(0.0, 1.0);
        let is_plinth = y0 < plinth_top_y;
        let plinth = if is_plinth { 450 } else { 0 };
        let inset = round(keep.batter_mm.max(0) as f64 * t) as i64 / 2;

        let w_out_bot = half_w - inset + plinth;
        let w_in_bot = (half_w - thickness - inset - plinth).max(100);
        let d_out_bot = half_d - inset + plinth;
        let d_in_bot = (half_d - thickness - inset - plinth).max(100);

        let u_faces_own = course_index % 2 == 0;
        let bond = if course_index % 2 == 0 { 0 } else { 475 };

        for face in [
            SquareFace::PosU,
            SquareFace::NegU,
            SquareFace::PosV,
            SquareFace::NegV,
        ] {
            let owns_corner = match face {
                SquareFace::PosU | SquareFace::NegU => u_faces_own,
                SquareFace::PosV | SquareFace::NegV => !u_faces_own,
            };
            let (a0, a1, d0, d1) = match face {
                SquareFace::PosU => (
                    -if owns_corner { d_out_bot } else { d_in_bot },
                    if owns_corner { d_out_bot } else { d_in_bot },
                    -w_out_bot,
                    -w_in_bot,
                ),
                SquareFace::NegU => (
                    -if owns_corner { d_out_bot } else { d_in_bot },
                    if owns_corner { d_out_bot } else { d_in_bot },
                    w_in_bot,
                    w_out_bot,
                ),
                SquareFace::PosV => (
                    -if owns_corner { w_out_bot } else { w_in_bot },
                    if owns_corner { w_out_bot } else { w_in_bot },
                    d_in_bot,
                    d_out_bot,
                ),
                SquareFace::NegV => (
                    -if owns_corner { w_out_bot } else { w_in_bot },
                    if owns_corner { w_out_bot } else { w_in_bot },
                    -d_out_bot,
                    -d_in_bot,
                ),
            };
            let joints = joint_positions(
                a0,
                a1,
                950,
                bond,
                element_seed,
                course_index,
                face.index() * 1_000,
            );
            let count = joints.len().saturating_sub(1);
            for (segment, pair) in joints.windows(2).enumerate() {
                let (a0b, a1b) = (pair[0], pair[1]);
                if a1b <= a0b {
                    continue;
                }
                let corners = square_face_corners(
                    keep.centre_mm,
                    keep.orientation_deg,
                    face,
                    a0b,
                    a1b,
                    a0b,
                    a1b,
                    d0,
                    d1,
                    d0,
                    d1,
                    y0,
                    y1,
                );
                if is_buried(&corners, terrain) {
                    continue;
                }
                let column = face.index() * 100_000 + segment as i32;
                push_custom_block(
                    out,
                    keep.id,
                    ElementKind::Keep,
                    course_index,
                    column as i64,
                    corners,
                    a1b - a0b,
                    y1 - y0,
                    element_seed,
                    stone_tint(hash01(element_seed, course_index as i64, column as i64, 21)),
                    (0.55 + 0.45 * t as f32).min(1.0),
                    BlockRole::Masonry,
                    [
                        false,
                        true,
                        true,
                        true,
                        owns_corner && segment == 0,
                        owns_corner && segment + 1 == count,
                    ],
                );
            }
        }

        course_index += 1;
        y0 = y1;
    }

    // Crenellated parapet: a ring of merlons sitting on the wall head.
    let inset_top = round(keep.batter_mm.max(0) as f64) as i64 / 2;
    let w_out_top = half_w - inset_top;
    let d_out_top = half_d - inset_top;
    let merlon_height = (keep.height_mm / 18).clamp(700, 1_400);
    let merlon_width = 1_150i64;
    let merlon_depth = thickness.clamp(300, 700);
    let gap = 800i64;
    let mut merlon_index = 0i32;
    let mut merlon = |u0: i64, u1: i64, v0: i64, v1: i64| {
        let corners = oriented_box_corners(
            keep.centre_mm,
            keep.orientation_deg,
            u0,
            u1,
            v0,
            v1,
            top_y,
            top_y + merlon_height,
        );
        push_custom_block(
            out,
            keep.id,
            ElementKind::Keep,
            course_index,
            merlon_index as i64,
            corners,
            u1 - u0,
            merlon_height,
            element_seed,
            stone_tint(hash01(element_seed, 7, merlon_index as i64, 22)),
            0.85,
            BlockRole::Masonry,
            [false, true, true, true, true, true],
        );
        merlon_index += 1;
    };
    let step = merlon_width + gap;
    for (span, north) in [(w_out_top, true), (w_out_top, false)] {
        let start = -span + merlon_width / 2 + 200;
        let end = span - merlon_width / 2 - 200;
        let mut centre = start;
        while centre <= end {
            let (v0, v1) = if north {
                (d_out_top - merlon_depth, d_out_top)
            } else {
                (-d_out_top, -d_out_top + merlon_depth)
            };
            merlon(centre - merlon_width / 2, centre + merlon_width / 2, v0, v1);
            centre += step;
        }
    }
    for (span, east) in [(d_out_top, true), (d_out_top, false)] {
        let start = -span + merlon_width / 2 + 200;
        let end = span - merlon_width / 2 - 200;
        let mut centre = start;
        while centre <= end {
            let (u0, u1) = if east {
                (w_out_top - merlon_depth, w_out_top)
            } else {
                (-w_out_top, -w_out_top + merlon_depth)
            };
            merlon(u0, u1, centre - merlon_width / 2, centre + merlon_width / 2);
            centre += step;
        }
    }
}

/// Lay out a roofed domestic range: four coursed walls, stepped triangular
/// gable ends above the eaves, and two coursed roof planes clipped against
/// higher-priority masonry so the roof never pokes through a parapet.
fn layout_range(
    range: &Range,
    document: &CastleDocument,
    terrain: &SiteTerrain,
    out: &mut Vec<Block>,
) {
    let half_l = (range.length_mm.max(4_000) / 2).max(2_000);
    let half_w = (range.width_mm.max(3_000) / 2).max(1_500);
    let thickness = range_wall_thickness_mm(range).clamp(400, half_w.saturating_sub(200));
    let (ground_min, ground_max) = ground_range_rect(
        range.centre_mm,
        range.length_mm,
        range.width_mm,
        range.orientation_deg,
        terrain,
    );
    let foundation_y = ground_min - 1_200;
    let eave_y = ground_max + range.wall_height_mm.max(1);
    let ridge_y = eave_y + range.ridge_rise_mm().max(0);
    let total_height = (ridge_y - foundation_y).max(1);
    let element_seed = range.seed ^ CASTLE_SEED.rotate_left(31);
    let pitch_tan = libm::tan((range.roof_pitch_deg.max(1) as f64).to_radians()).max(0.05);

    let l_out = half_l;
    let l_in = (half_l - thickness).max(100);
    let w_out = half_w;
    let w_in = (half_w - thickness).max(100);

    let course_nominal = 430i64;
    let mut course_index = 0i32;

    // Body walls, foundation to the eaves.
    let mut y0 = foundation_y;
    while y0 < eave_y {
        let jitter = (hash01(element_seed, i64::from(course_index), 3, 0) - 0.5) * 30.0;
        let mut y1 = y0 + course_nominal + jitter as i64;
        if y1 > eave_y || eave_y - y1 < course_nominal / 2 {
            y1 = eave_y;
        }
        if y1 <= y0 {
            break;
        }
        let mid_y = (y0 + y1) / 2;
        let t = ((mid_y - foundation_y) as f64 / total_height as f64).clamp(0.0, 1.0);
        let u_faces_own = course_index % 2 == 0;
        let bond = if course_index % 2 == 0 { 0 } else { 475 };
        for face in [
            SquareFace::PosU,
            SquareFace::NegU,
            SquareFace::PosV,
            SquareFace::NegV,
        ] {
            let owns_corner = match face {
                SquareFace::PosU | SquareFace::NegU => u_faces_own,
                SquareFace::PosV | SquareFace::NegV => !u_faces_own,
            };
            let (a0, a1, d0, d1) = match face {
                SquareFace::PosU => (
                    -if owns_corner { w_out } else { w_in },
                    if owns_corner { w_out } else { w_in },
                    -l_out,
                    -l_in,
                ),
                SquareFace::NegU => (
                    -if owns_corner { w_out } else { w_in },
                    if owns_corner { w_out } else { w_in },
                    l_in,
                    l_out,
                ),
                SquareFace::PosV => (
                    -if owns_corner { l_out } else { l_in },
                    if owns_corner { l_out } else { l_in },
                    w_in,
                    w_out,
                ),
                SquareFace::NegV => (
                    -if owns_corner { l_out } else { l_in },
                    if owns_corner { l_out } else { l_in },
                    -w_out,
                    -w_in,
                ),
            };
            let joints = joint_positions(
                a0,
                a1,
                950,
                bond,
                element_seed,
                course_index,
                face.index() * 1_000,
            );
            let count = joints.len().saturating_sub(1);
            for (segment, pair) in joints.windows(2).enumerate() {
                let (a0b, a1b) = (pair[0], pair[1]);
                if a1b <= a0b {
                    continue;
                }
                let corners = square_face_corners(
                    range.centre_mm,
                    range.orientation_deg,
                    face,
                    a0b,
                    a1b,
                    a0b,
                    a1b,
                    d0,
                    d1,
                    d0,
                    d1,
                    y0,
                    y1,
                );
                if is_buried(&corners, terrain) {
                    continue;
                }
                let column = face.index() * 100_000 + segment as i32;
                push_custom_block(
                    out,
                    range.id,
                    ElementKind::Range,
                    course_index,
                    column as i64,
                    corners,
                    a1b - a0b,
                    y1 - y0,
                    element_seed,
                    stone_tint(hash01(element_seed, course_index as i64, column as i64, 23)),
                    (0.55 + 0.45 * t as f32).min(1.0),
                    BlockRole::Masonry,
                    [
                        false,
                        true,
                        true,
                        true,
                        owns_corner && segment == 0,
                        owns_corner && segment + 1 == count,
                    ],
                );
            }
        }
        course_index += 1;
        y0 = y1;
    }

    // Gable ends: stepped courses that narrow as they rise to the ridge.
    let mut y0 = eave_y;
    while y0 < ridge_y {
        let jitter = (hash01(element_seed, i64::from(course_index), 4, 0) - 0.5) * 24.0;
        let mut y1 = y0 + course_nominal + jitter as i64;
        if y1 > ridge_y || ridge_y - y1 < course_nominal / 2 {
            y1 = ridge_y;
        }
        if y1 <= y0 {
            break;
        }
        let run_half = (((ridge_y - y1) as f64 / pitch_tan) as i64).clamp(0, half_w);
        if run_half < 300 {
            break;
        }
        let bond = if course_index % 2 == 0 { 0 } else { 475 };
        for face in [SquareFace::PosU, SquareFace::NegU] {
            let (d0, d1) = if matches!(face, SquareFace::PosU) {
                (-l_out, -l_in)
            } else {
                (l_in, l_out)
            };
            let joints = joint_positions(
                -run_half,
                run_half,
                950,
                bond,
                element_seed,
                course_index,
                face.index() * 1_000,
            );
            let count = joints.len().saturating_sub(1);
            for (segment, pair) in joints.windows(2).enumerate() {
                let (a0b, a1b) = (pair[0], pair[1]);
                if a1b <= a0b {
                    continue;
                }
                let corners = square_face_corners(
                    range.centre_mm,
                    range.orientation_deg,
                    face,
                    a0b,
                    a1b,
                    a0b,
                    a1b,
                    d0,
                    d1,
                    d0,
                    d1,
                    y0,
                    y1,
                );
                if is_buried(&corners, terrain) {
                    continue;
                }
                let column = face.index() * 100_000 + segment as i32;
                push_custom_block(
                    out,
                    range.id,
                    ElementKind::Range,
                    course_index,
                    column as i64,
                    corners,
                    a1b - a0b,
                    y1 - y0,
                    element_seed,
                    stone_tint(hash01(element_seed, course_index as i64, column as i64, 23)),
                    0.8,
                    BlockRole::Masonry,
                    [
                        false,
                        true,
                        true,
                        true,
                        segment == 0 && count > 0,
                        segment + 1 == count,
                    ],
                );
            }
        }
        course_index += 1;
        y0 = y1;
    }

    // Two pitched roof planes in coursed strips. Each u strip is divided along
    // the slope from ridge to eave into coursed slate segments; only the
    // segments that actually overlap a higher-priority wall, tower or keep
    // (in plan *and* elevation) are dropped. A curtain abutting the eave trims
    // the outer courses and leaves the rest of the roof intact instead of
    // deleting the whole slope.
    let roof_thickness = 180i64;
    let strip = 850i64;
    const SLOPE_SEGMENTS: i64 = 7;
    let roof_tint = match range.kind {
        RangeKind::Chapel => 0.62,
        RangeKind::Hall => 0.5,
        RangeKind::Kitchen => 0.46,
    };
    let roof_seed = element_seed ^ 0x5A5A_1234_5678_9ABC;
    for sign in [-1i64, 1] {
        let (v0, v1, y0_roof, y1_roof) = if sign > 0 {
            (0, half_w, ridge_y, eave_y)
        } else {
            (-half_w, 0, eave_y, ridge_y)
        };
        let mut u0 = -half_l;
        let mut roof_index = 0i32;
        while u0 < half_l {
            let u1 = (u0 + strip).min(half_l);
            let u_mid = (u0 + u1) / 2;
            for segment in 0..SLOPE_SEGMENTS {
                let s0 = segment as f64 / SLOPE_SEGMENTS as f64;
                let s1 = (segment + 1) as f64 / SLOPE_SEGMENTS as f64;
                let s_mid = (s0 + s1) * 0.5;
                let v_at = |s: f64| -> i64 { v0 + round((v1 - v0) as f64 * s) as i64 };
                let y_at =
                    |s: f64| -> i64 { y0_roof + round((y1_roof - y0_roof) as f64 * s) as i64 };

                let blocked = [s0, s_mid, s1].iter().any(|&s| {
                    let (x, z) = local_to_world(
                        range.centre_mm,
                        range.orientation_deg,
                        u_mid as f64,
                        v_at(s) as f64,
                    );
                    range_roof_blocked(document, range.id, terrain, x, z, y_at(s))
                });
                if blocked {
                    continue;
                }

                let corners = roof_slab_corners(
                    range.centre_mm,
                    range.orientation_deg,
                    u0,
                    u1,
                    v_at(s0),
                    v_at(s1),
                    y_at(s0),
                    y_at(s1),
                    roof_thickness,
                );
                push_custom_block(
                    out,
                    range.id,
                    ElementKind::Range,
                    course_index,
                    sign * 1_000_000 + i64::from(roof_index),
                    corners,
                    u1 - u0,
                    roof_thickness,
                    roof_seed,
                    roof_tint,
                    0.7,
                    BlockRole::Roof,
                    [false, true, true, true, u0 <= -half_l, u1 >= half_l],
                );
                roof_index += 1;
            }
            u0 = u1;
        }
    }
}

/// Sloped roof strip: `y0` at `v0`, `y1` at `v1`, with a vertical thickness.
#[allow(clippy::too_many_arguments)]
fn roof_slab_corners(
    centre_mm: [i64; 2],
    orientation_deg: i32,
    u0: i64,
    u1: i64,
    v0: i64,
    v1: i64,
    y0: i64,
    y1: i64,
    thickness: i64,
) -> [[i64; 3]; 8] {
    let p = |u: i64, v: i64, y: i64| -> [i64; 3] {
        let (x, z) = local_to_world(centre_mm, orientation_deg, u as f64, v as f64);
        [round(x) as i64, y, round(z) as i64]
    };
    [
        p(u0, v0, y0),
        p(u1, v0, y0),
        p(u1, v1, y1),
        p(u0, v1, y1),
        p(u0, v0, y0 + thickness),
        p(u1, v0, y0 + thickness),
        p(u1, v1, y1 + thickness),
        p(u0, v1, y1 + thickness),
    ]
}

/// True when a roof point lies inside a higher-priority wall, tower or keep
/// footprint at that elevation. The plan test is expanded a little so a roof
/// edge stops cleanly short of the masonry, and the elevation test lets a roof
/// pass above a lower wall instead of being clipped by it.
fn range_roof_blocked(
    document: &CastleDocument,
    self_id: u64,
    terrain: &SiteTerrain,
    x: f64,
    z: f64,
    y: i64,
) -> bool {
    const MARGIN_MM: f64 = 250.0;
    for element in document.elements() {
        if element.id() == self_id {
            continue;
        }
        match element {
            Element::BatteredWall(wall) => {
                let start = terrain.height_mm(wall.start_mm[0], wall.start_mm[1]);
                let end = terrain.height_mm(wall.end_mm[0], wall.end_mm[1]);
                let mid = terrain.height_mm(
                    (wall.start_mm[0] + wall.end_mm[0]) / 2,
                    (wall.start_mm[1] + wall.end_mm[1]) / 2,
                );
                if y > start.max(end).max(mid) + wall.height_mm {
                    continue;
                }
                let ax = wall.start_mm[0] as f64;
                let az = wall.start_mm[1] as f64;
                let bx = wall.end_mm[0] as f64;
                let bz = wall.end_mm[1] as f64;
                let reach = wall.thickness_mm as f64 * 0.5 + MARGIN_MM;
                if point_segment_distance(x, z, ax, az, bx, bz) <= reach {
                    return true;
                }
            }
            Element::RoundTower(tower) => {
                if y > terrain.height_mm(tower.centre_mm[0], tower.centre_mm[1]) + tower.height_mm {
                    continue;
                }
                let dx = x - tower.centre_mm[0] as f64;
                let dz = z - tower.centre_mm[1] as f64;
                let reach = tower.radius_mm as f64 + tower.plinth_projection_mm as f64 + MARGIN_MM;
                if dx * dx + dz * dz <= reach * reach {
                    return true;
                }
            }
            Element::Gatehouse(gatehouse) => {
                for tower in gatehouse.flank_towers() {
                    if y > terrain.height_mm(tower.centre_mm[0], tower.centre_mm[1])
                        + tower.height_mm
                    {
                        continue;
                    }
                    let dx = x - tower.centre_mm[0] as f64;
                    let dz = z - tower.centre_mm[1] as f64;
                    let reach = tower.radius_mm as f64 + MARGIN_MM;
                    if dx * dx + dz * dz <= reach * reach {
                        return true;
                    }
                }
            }
            Element::Keep(keep) => {
                if y > terrain.height_mm(keep.centre_mm[0], keep.centre_mm[1]) + keep.height_mm {
                    continue;
                }
                let (u, v) = world_to_local(keep.centre_mm, keep.orientation_deg, x, z);
                let half_w = keep.width_mm as f64 * 0.5 + MARGIN_MM;
                let half_d = keep.depth_mm as f64 * 0.5 + MARGIN_MM;
                if u.abs() <= half_w && v.abs() <= half_d {
                    return true;
                }
            }
            Element::Range(_) | Element::Ditch(_) => {}
        }
    }
    false
}

fn world_to_local(centre_mm: [i64; 2], orientation_deg: i32, x: f64, z: f64) -> (f64, f64) {
    let o = orientation_deg as f64 * std::f64::consts::PI / 180.0;
    let (oc, os) = (cos(o), sin(o));
    let dx = x - centre_mm[0] as f64;
    let dz = z - centre_mm[1] as f64;
    (dx * oc + dz * os, -dx * os + dz * oc)
}

fn point_segment_distance(px: f64, pz: f64, ax: f64, az: f64, bx: f64, bz: f64) -> f64 {
    let abx = bx - ax;
    let abz = bz - az;
    let denom = abx * abx + abz * abz;
    let t = if denom <= f64::EPSILON {
        0.0
    } else {
        (((px - ax) * abx + (pz - az) * abz) / denom).clamp(0.0, 1.0)
    };
    let dx = ax + abx * t - px;
    let dz = az + abz * t - pz;
    sqrt(dx * dx + dz * dz)
}

/// Which straight face of a square tower a block belongs to. The `d` axis is
/// always the right-hand axis of the run so the shared [`Block`] winding stays
/// outward on every face.
#[derive(Clone, Copy)]
enum SquareFace {
    PosU,
    NegU,
    PosV,
    NegV,
}

impl SquareFace {
    fn index(self) -> i32 {
        match self {
            Self::PosU => 0,
            Self::NegU => 1,
            Self::PosV => 2,
            Self::NegV => 3,
        }
    }

    fn local_uv(self, a: i64, d: i64) -> (i64, i64) {
        match self {
            // The u-faces run along v; their thickness axis points along -u.
            Self::PosU | Self::NegU => (-d, a),
            // The v-faces run along u; their thickness axis points along +v.
            Self::PosV | Self::NegV => (a, d),
        }
    }
}

/// Solve a single run of shared, jittered joints between `start_mm` and
/// `end_mm`. `bond` shifts the whole joint pattern so alternate courses break
/// stack bond without ever moving the two end joints.
fn joint_positions(
    start_mm: i64,
    end_mm: i64,
    nominal_mm: i64,
    bond_mm: i64,
    seed: u64,
    course: i32,
    column_base: i32,
) -> Vec<i64> {
    if end_mm <= start_mm {
        return vec![start_mm, end_mm];
    }
    let mut joints = vec![start_mm];
    let mut pos = start_mm + bond_mm;
    let mut column = 0i32;
    while pos + nominal_mm < end_mm {
        let vary =
            (hash01(seed, i64::from(course), i64::from(column_base + column), 7) - 0.5) * 420.0;
        pos += nominal_mm + vary as i64;
        let previous = *joints.last().unwrap();
        if pos <= previous + 250 {
            pos = previous + 500;
        }
        if pos >= end_mm - 200 {
            break;
        }
        let jitter = stone_jitter(seed, course, column_base + column, 1);
        joints.push(pos + jitter);
        column += 1;
    }
    joints.push(end_mm);
    joints
}

/// Map a joint position from the bottom run span onto the top run span so a
/// battered face keeps a straight, quartered edge.
fn map_run(a: i64, a_start: i64, a_end: i64, top_start: i64, top_end: i64) -> i64 {
    if a_end == a_start {
        return top_start;
    }
    top_start
        + round((a - a_start) as f64 * (top_end - top_start) as f64 / (a_end - a_start) as f64)
            as i64
}

/// Scale a plan coordinate from one course radius to the next.
fn map_radius(value: i64, from: i64, to: i64) -> i64 {
    if from == 0 {
        0
    } else {
        round(value as f64 * to as f64 / from as f64) as i64
    }
}

#[allow(clippy::too_many_arguments)]
fn square_face_corners(
    centre_mm: [i64; 2],
    orientation_deg: i32,
    face: SquareFace,
    a0_bot: i64,
    a1_bot: i64,
    a0_top: i64,
    a1_top: i64,
    d0_bot: i64,
    d1_bot: i64,
    d0_top: i64,
    d1_top: i64,
    y0: i64,
    y1: i64,
) -> [[i64; 3]; 8] {
    let p = |a: i64, d: i64, y: i64| -> [i64; 3] {
        let (u, v) = face.local_uv(a, d);
        shaped_local_point(centre_mm, orientation_deg, u, v, y)
    };
    [
        p(a0_bot, d0_bot, y0),
        p(a1_bot, d0_bot, y0),
        p(a1_bot, d1_bot, y0),
        p(a0_bot, d1_bot, y0),
        p(a0_top, d0_top, y1),
        p(a1_top, d0_top, y1),
        p(a1_top, d1_top, y1),
        p(a0_top, d1_top, y1),
    ]
}

#[allow(clippy::too_many_arguments)]
fn dshaped_arc_corners(
    centre_mm: [i64; 2],
    a0: f64,
    a1: f64,
    r_out_bot: i64,
    r_in_bot: i64,
    r_out_top: i64,
    r_in_top: i64,
    orientation_deg: i32,
    y0: i64,
    y1: i64,
) -> [[i64; 3]; 8] {
    let p = |angle: f64, radius: i64, y: i64| -> [i64; 3] {
        shaped_local_point(
            centre_mm,
            orientation_deg,
            round(cos(angle) * radius as f64) as i64,
            round(sin(angle) * radius as f64) as i64,
            y,
        )
    };
    [
        p(a0, r_out_bot, y0),
        p(a1, r_out_bot, y0),
        p(a1, r_in_bot, y0),
        p(a0, r_in_bot, y0),
        p(a0, r_out_top, y1),
        p(a1, r_out_top, y1),
        p(a1, r_in_top, y1),
        p(a0, r_in_top, y1),
    ]
}

#[allow(clippy::too_many_arguments)]
fn shaped_local_point(
    centre_mm: [i64; 2],
    orientation_deg: i32,
    u: i64,
    v: i64,
    y: i64,
) -> [i64; 3] {
    let o = orientation_deg as f64 * std::f64::consts::PI / 180.0;
    let (oc, os) = (cos(o), sin(o));
    let x = centre_mm[0] as f64 + u as f64 * oc - v as f64 * os;
    let z = centre_mm[1] as f64 + u as f64 * os + v as f64 * oc;
    [round(x) as i64, y, round(z) as i64]
}

#[allow(clippy::too_many_arguments)]
fn dshaped_rear_corners(
    centre_mm: [i64; 2],
    orientation_deg: i32,
    v0_bot: i64,
    v1_bot: i64,
    v0_top: i64,
    v1_top: i64,
    u_out: i64,
    u_in: i64,
    y0: i64,
    y1: i64,
) -> [[i64; 3]; 8] {
    [
        shaped_local_point(centre_mm, orientation_deg, u_out, v0_bot, y0),
        shaped_local_point(centre_mm, orientation_deg, u_out, v1_bot, y0),
        shaped_local_point(centre_mm, orientation_deg, u_in, v1_bot, y0),
        shaped_local_point(centre_mm, orientation_deg, u_in, v0_bot, y0),
        shaped_local_point(centre_mm, orientation_deg, u_out, v0_top, y1),
        shaped_local_point(centre_mm, orientation_deg, u_out, v1_top, y1),
        shaped_local_point(centre_mm, orientation_deg, u_in, v1_top, y1),
        shaped_local_point(centre_mm, orientation_deg, u_in, v0_top, y1),
    ]
}

/// Lay out the gatehouse: two flanking drum towers, solid passage side walls
/// between the passage and the towers, and a bridging chamber over a clear
/// passage. The bridge is embedded into both towers, so no floating gap can
/// appear, and every course is laid in running-bond ashlar across its width.
fn layout_gatehouse(gatehouse: &Gatehouse, terrain: &SiteTerrain, out: &mut Vec<Block>) {
    let towers = gatehouse.flank_towers();
    // The towers' plinth projects outward at ground level; the bridge has to
    // span past that projection to stay buried in the drums.
    let plinth_offset = towers[0].plinth_projection_mm.max(0);
    for tower in &towers {
        layout_tower(tower, terrain, out);
    }

    let ground = terrain.height_mm(gatehouse.centre_mm[0], gatehouse.centre_mm[1]);
    let passage_half = gatehouse.passage_width_mm.max(1) / 2;
    let passage_top = ground + gatehouse.passage_height_mm.max(1);
    let top = ground + gatehouse.height_mm.max(1);
    if top <= passage_top {
        return;
    }

    let dir = gatehouse.direction();
    let right = [-dir[1], dir[0]];
    // Keep the central bridge tucked cleanly between the two flanking drums so
    // the rounded towers project proud on both approach and bailey sides.
    let max_tucked_half_depth = (gatehouse.tower_radius_mm.max(1_000) * 65) / 100;
    let half_depth = (gatehouse.depth_mm.max(1) / 2).min(max_tucked_half_depth);
    // Reach 500 mm past the passage edge plus the tower plinth projection so
    // both ends of the bridge bite into the drum shells. The tower faces recede
    // with batter as they rise, so this stays embedded at every course and the
    // old mid-air gap between bridge and towers cannot open up again.
    let half_span = passage_half + 500 + plinth_offset;
    let element_seed = gatehouse.seed ^ CASTLE_SEED.rotate_left(23);
    let course_nominal = 450i64;
    let nominal_block = 900i64;
    let total_height = (top - passage_top).max(1);

    let point = |u: i64, v: i64, y: i64| -> [i64; 3] {
        let x = gatehouse.centre_mm[0] as f64 + dir[0] * u as f64 + right[0] * v as f64;
        let z = gatehouse.centre_mm[1] as f64 + dir[1] * u as f64 + right[1] * v as f64;
        [round(x) as i64, y, round(z) as i64]
    };

    // Lower passage side walls: fill the band between the clear passage edge
    // and the drum towers, from ground to the passage crown, on both sides.
    // The wall rises in its own courses so the passage stays fully open.
    let mut y1 = passage_top;
    let mut side_course = 0i32;
    while y1 > ground {
        let y0 = (y1 - course_nominal).max(ground);
        if y0 >= y1 {
            break;
        }
        let course_index = -1 - side_course;
        let bond = if side_course % 2 == 0 {
            0
        } else {
            nominal_block / 2
        };
        for (side_index, sign) in [-1i64, 1].into_iter().enumerate() {
            let (d0, d1) = if sign < 0 {
                (-half_span, -passage_half)
            } else {
                (passage_half, half_span)
            };
            let joints = joint_positions(
                -half_depth,
                half_depth,
                nominal_block,
                bond,
                element_seed,
                course_index,
                20_000 + side_index as i32 * 1_000,
            );
            for pair in joints.windows(2) {
                let (u0, u1) = (pair[0], pair[1]);
                if u1 <= u0 {
                    continue;
                }
                let corners = [
                    point(u0, d0, y0),
                    point(u1, d0, y0),
                    point(u1, d1, y0),
                    point(u0, d1, y0),
                    point(u0, d0, y1),
                    point(u1, d0, y1),
                    point(u1, d1, y1),
                    point(u0, d1, y1),
                ];
                if is_buried(&corners, terrain) {
                    continue;
                }
                let column = u0 + half_depth;
                out.push(Block {
                    element_id: gatehouse.id,
                    kind: ElementKind::Gatehouse,
                    course_index,
                    run_pos_mm: (side_index as i64 + 1) * 1_000_000 + column,
                    build_index: 0,
                    corners_mm: corners,
                    block_width_mm: (u1 - u0).max(1),
                    block_height_mm: (y1 - y0).max(1),
                    tint: stone_tint(hash01(
                        element_seed,
                        course_index as i64,
                        side_index as i64 * 1_000 + column,
                        11,
                    )),
                    age: (0.55 + 0.45 * (y0 - ground) as f32 / total_height as f32).min(1.0),
                    jitter_seed: hash01(
                        element_seed,
                        course_index as i64,
                        side_index as i64 * 1_000 + column,
                        12,
                    ),
                    // Every vertical face is exposed: the outward face is
                    // visible past the drum at the facades and buried inside
                    // it near the passage. Top is always exposed so the rising
                    // wall reads as solid during construction.
                    expose: [false, true, true, true, true, true],
                    role: BlockRole::Masonry,
                });
            }
        }
        y1 = y0;
        side_course += 1;
    }

    // Bridging chamber above the passage. Each course is a row of running-bond
    // ashlar blocks across the full lateral span, so the front and rear faces
    // show real coursed stonework rather than one plank per bed.
    let mut course_index = 0i32;
    let mut y0 = passage_top;
    while y0 < top {
        let jitter = (hash01(element_seed, i64::from(course_index), 3, 0) - 0.5) * 40.0;
        let mut y1 = y0 + course_nominal + jitter as i64;
        if y1 > top || top - y1 < course_nominal / 2 {
            y1 = top;
        }
        if y1 <= y0 {
            break;
        }
        let t = ((y0 + y1) / 2 - passage_top) as f64 / total_height as f64;
        let lintel = course_index == 0;
        // Stepped lintel trim: the two courses over the passage project past
        // the facade, then step back to the wall plane, forming a corbelled
        // band across the top of the gateway.
        let corbel = match course_index {
            0 => 180i64,
            1 => 90i64,
            _ => 0,
        };
        let u0 = -half_depth - corbel;
        let u1 = half_depth + corbel;
        let bond = if course_index % 2 == 0 {
            0
        } else {
            nominal_block / 2
        };
        let joints = joint_positions(
            -half_span,
            half_span,
            nominal_block,
            bond,
            element_seed,
            course_index,
            40_000,
        );
        for (column, pair) in joints.windows(2).enumerate() {
            let (v0, v1) = (pair[0], pair[1]);
            if v1 <= v0 {
                continue;
            }
            let corners = [
                point(u0, v0, y0),
                point(u1, v0, y0),
                point(u1, v1, y0),
                point(u0, v1, y0),
                point(u0, v0, y1),
                point(u1, v0, y1),
                point(u1, v1, y1),
                point(u0, v1, y1),
            ];
            if is_buried(&corners, terrain) {
                continue;
            }
            out.push(Block {
                element_id: gatehouse.id,
                kind: ElementKind::Gatehouse,
                course_index,
                run_pos_mm: v0 + half_span,
                build_index: 0,
                corners_mm: corners,
                block_width_mm: (v1 - v0).max(1),
                block_height_mm: (y1 - y0).max(1),
                tint: stone_tint(hash01(element_seed, course_index as i64, column as i64, 11)),
                age: (0.55 + 0.45 * t as f32).min(1.0),
                jitter_seed: hash01(element_seed, course_index as i64, column as i64, 12),
                // The lintel soffit is the visible passage ceiling. The side
                // ends are buried in the drums near the passage and emerge past
                // them at the facades, so all vertical faces stay exposed.
                expose: [lintel, true, true, true, true, true],
                role: if lintel {
                    BlockRole::Lintel
                } else {
                    BlockRole::Masonry
                },
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
    r_out_bot: i64,
    r_in_bot: i64,
    r_out_top: i64,
    r_in_top: i64,
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
        p(a0, r_out_bot, y0),
        p(a1, r_out_bot, y0),
        p(a1, r_in_bot, y0),
        p(a0, r_in_bot, y0),
        p(a0, r_out_top, y1),
        p(a1, r_out_top, y1),
        p(a1, r_in_top, y1),
        p(a0, r_in_top, y1),
    ]
}

/// Wall face offsets `(d0, d1)` at elevation `y`, with the plinth projection
/// held constant across a course so the ledge steps at a course boundary.
fn wall_face_offsets(
    wall: &BatteredWall,
    foundation_y: i64,
    total_height: i64,
    y: i64,
    plinth: i64,
) -> (i64, i64) {
    let t = ((y - foundation_y) as f64 / total_height as f64).clamp(0.0, 1.0);
    let batter_inset = round(wall.batter_mm as f64 * t) as i64;
    let d0 = -wall.thickness_mm / 2 + batter_inset / 2 - plinth;
    let mut d1 = wall.thickness_mm / 2 - batter_inset / 2 + plinth;
    if d1 <= d0 {
        d1 = d0 + 100;
    }
    (d0, d1)
}

/// Tower shell radii `(r_out, r_in)` at elevation `y`, with the plinth
/// projection held constant across a course.
fn tower_face_radii(
    tower: &RoundTower,
    thickness: i64,
    foundation_y: i64,
    total_height: i64,
    y: i64,
    plinth: i64,
) -> (i64, i64) {
    let t = ((y - foundation_y) as f64 / total_height as f64).clamp(0.0, 1.0);
    let batter_inset = round(tower.batter_mm as f64 * t) as i64;
    let r_out = (tower.radius_mm - batter_inset / 2 + plinth).max(400);
    let r_in = (tower.radius_mm - thickness - batter_inset / 2 - plinth).max(80);
    (r_out, r_in)
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
    use crate::fixtures::{m0a_showcase_document, m0b_showcase_document, m1_edwardian_document};

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

    /// Collect the actual bottom and top elevation of every emitted course for
    /// one element. All stones in a course share the same bed planes.
    fn emitted_courses(
        blocks: &[Block],
        element_id: u64,
    ) -> std::collections::BTreeMap<i32, (i64, i64)> {
        let mut courses: std::collections::BTreeMap<i32, (i64, i64)> =
            std::collections::BTreeMap::new();
        for block in blocks
            .iter()
            .filter(|b| b.element_id == element_id && b.role != BlockRole::Jamb)
        {
            let y0 = block.corners_mm[0][1];
            let y1 = block.corners_mm[4][1];
            courses
                .entry(block.course_index)
                .and_modify(|(low, high)| {
                    *low = (*low).min(y0);
                    *high = (*high).max(y1);
                })
                .or_insert((y0, y1));
        }
        courses
    }

    #[test]
    fn stacked_courses_share_watertight_bed_joints() {
        let terrain = SiteTerrain::m0a_promontory();
        let doc = m0b_showcase_document();
        let blocks = generate_blocks(&doc, &terrain);
        let mut wall_pairs = 0;
        let mut tower_pairs = 0;

        for element in doc.elements() {
            match element {
                Element::BatteredWall(wall) => {
                    let (foundation_y, plinth_top_y, top_y) =
                        junction::wall_elevations(wall, &terrain);
                    let total_height = (top_y - foundation_y).max(1);
                    let courses = emitted_courses(&blocks, wall.id);
                    for ((course, &(y0, _)), (next_course, &(next_y0, _))) in
                        courses.iter().zip(courses.iter().skip(1))
                    {
                        if *next_course != *course + 1 {
                            continue;
                        }
                        let plinth = y0 < plinth_top_y;
                        let next_plinth = next_y0 < plinth_top_y;
                        // The capped plinth ledge is a deliberate outward step.
                        if plinth != next_plinth {
                            continue;
                        }
                        let top = courses[course].1;
                        assert_eq!(top, next_y0, "wall {} course {} bed gap", wall.id, course);
                        let (d0_top, d1_top) = wall_face_offsets(
                            wall,
                            foundation_y,
                            total_height,
                            top,
                            if plinth { wall.plinth_projection_mm } else { 0 },
                        );
                        let (d0_bot, d1_bot) = wall_face_offsets(
                            wall,
                            foundation_y,
                            total_height,
                            next_y0,
                            if next_plinth {
                                wall.plinth_projection_mm
                            } else {
                                0
                            },
                        );
                        assert!(
                            (d0_top - d0_bot).abs() <= 1 && (d1_top - d1_bot).abs() <= 1,
                            "wall {} course {course}->{next_course}: face step {d0_top}/{d1_top} vs {d0_bot}/{d1_bot}",
                            wall.id
                        );
                        wall_pairs += 1;
                    }
                }
                Element::RoundTower(tower) => {
                    let (foundation_y, plinth_top_y, top_y) =
                        junction::tower_elevations(tower, &terrain);
                    let total_height = (top_y - foundation_y).max(1);
                    let radius = tower.radius_mm.max(500);
                    let thickness = tower.thickness_mm.clamp(300, radius);
                    let courses = emitted_courses(&blocks, tower.id);
                    for ((course, &(y0, _)), (next_course, &(next_y0, _))) in
                        courses.iter().zip(courses.iter().skip(1))
                    {
                        if *next_course != *course + 1 {
                            continue;
                        }
                        let plinth = y0 < plinth_top_y;
                        let next_plinth = next_y0 < plinth_top_y;
                        if plinth != next_plinth {
                            continue;
                        }
                        let top = courses[course].1;
                        assert_eq!(top, next_y0, "tower {} course {} bed gap", tower.id, course);
                        let (r_out_top, r_in_top) = tower_face_radii(
                            tower,
                            thickness,
                            foundation_y,
                            total_height,
                            top,
                            if plinth {
                                tower.plinth_projection_mm
                            } else {
                                0
                            },
                        );
                        let (r_out_bot, r_in_bot) = tower_face_radii(
                            tower,
                            thickness,
                            foundation_y,
                            total_height,
                            next_y0,
                            if next_plinth {
                                tower.plinth_projection_mm
                            } else {
                                0
                            },
                        );
                        assert!(
                            (r_out_top - r_out_bot).abs() <= 1
                                && (r_in_top - r_in_bot).abs() <= 1,
                            "tower {} course {course}->{next_course}: radius step {r_out_top}/{r_in_top} vs {r_out_bot}/{r_in_bot}",
                            tower.id
                        );
                        tower_pairs += 1;
                    }
                }
                Element::Gatehouse(_)
                | Element::Ditch(_)
                | Element::Keep(_)
                | Element::Range(_) => {}
            }
        }

        assert!(
            wall_pairs >= 10,
            "only checked {wall_pairs} wall bed joints"
        );
        assert!(
            tower_pairs >= 10,
            "only checked {tower_pairs} tower bed joints"
        );
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

    #[test]
    fn every_exposed_face_points_outward_for_all_element_kinds() {
        let cases = [
            (
                SiteTerrain::for_site("coastal_headland")
                    .expect("coastal_headland is a curated site"),
                m1_edwardian_document(),
                vec![
                    ElementKind::BatteredWall,
                    ElementKind::RoundTower,
                    ElementKind::Gatehouse,
                ],
            ),
            (
                SiteTerrain::for_site("lake_island").expect("lake_island is a curated site"),
                crate::fixtures::m2_baltic_document(),
                vec![
                    ElementKind::BatteredWall,
                    ElementKind::RoundTower,
                    ElementKind::Gatehouse,
                    ElementKind::Keep,
                    ElementKind::Range,
                ],
            ),
        ];
        for (terrain, doc, required) in cases {
            let blocks = generate_blocks(&doc, &terrain);
            assert!(!blocks.is_empty());

            let mut kinds = std::collections::BTreeSet::new();
            for block in &blocks {
                kinds.insert(block.kind);
                let centre = block.centre_mm();
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
                    let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
                    let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
                    let normal = [
                        e1[1] * e2[2] - e1[2] * e2[1],
                        e1[2] * e2[0] - e1[0] * e2[2],
                        e1[0] * e2[1] - e1[1] * e2[0],
                    ];
                    // The innermost splay jamb step tapers to a line, so its cap is
                    // a triangle stored in a quad slot. One triangle is degenerate
                    // and has no direction; the other still caps the stone.
                    if normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2] <= 1.0
                    {
                        continue;
                    }
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
                    let dot =
                        normal[0] * outward[0] + normal[1] * outward[1] + normal[2] * outward[2];
                    assert!(
                        dot > 1.0,
                        "{:?} block {} face {face:?} points inward (dot {dot})",
                        block.kind,
                        block.element_id
                    );
                }
            }
            for kind in required {
                assert!(kinds.contains(&kind), "no {kind:?} blocks were checked");
            }
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

    fn block_bounds(block: &Block) -> ([f64; 3], [f64; 3]) {
        let mut min = [f64::MAX; 3];
        let mut max = [f64::MIN; 3];
        for corner in block.corners_mm {
            for axis in 0..3 {
                min[axis] = min[axis].min(corner[axis] as f64);
                max[axis] = max[axis].max(corner[axis] as f64);
            }
        }
        (min, max)
    }

    fn assert_blocks_valid(blocks: &[Block], label: &str) {
        assert!(!blocks.is_empty(), "{label} produced no blocks");
        for block in blocks {
            let (min, max) = block_bounds(block);
            let volume = (max[0] - min[0]) * (max[1] - min[1]) * (max[2] - min[2]);
            assert!(volume > 0.0, "{label} produced a zero-volume block");
            assert!(volume.is_finite(), "{label} produced a non-finite block");
            for corner in block.corners_mm {
                assert!(corner[0].abs() < 500_000_000);
                assert!(corner[1].abs() < 500_000_000);
                assert!(corner[2].abs() < 500_000_000);
            }
        }
    }

    #[test]
    fn square_and_dshaped_towers_have_watertight_positive_volume_masonry() {
        let terrain = SiteTerrain::m0a_promontory();
        for (id, plan, orientation) in [
            (1u64, TowerPlan::Square, 0i32),
            (2, TowerPlan::Square, 37),
            (3, TowerPlan::DShaped, 0),
            (4, TowerPlan::DShaped, 145),
        ] {
            let mut doc = CastleDocument::new();
            let mut tower = RoundTower::new(id, "Shaped tower", [0, 0], 3_500, 12_000);
            tower.plan = plan;
            tower.orientation_deg = orientation;
            doc.add_tower(tower);
            let blocks = generate_blocks(&doc, &terrain);
            assert_blocks_valid(&blocks, &format!("{plan:?} tower"));
            // Courses must not leave daylight between stacked blocks.
            let mut courses = std::collections::BTreeMap::new();
            for block in &blocks {
                let (min, max) = block_bounds(block);
                courses
                    .entry(block.course_index)
                    .and_modify(|(low, high): &mut (f64, f64)| {
                        *low = low.min(min[1]);
                        *high = high.max(max[1]);
                    })
                    .or_insert((min[1], max[1]));
            }
            let ordered: Vec<(f64, f64)> = courses.values().copied().collect();
            for pair in ordered.windows(2) {
                assert!(
                    (pair[0].1 - pair[1].0).abs() <= 1.0,
                    "bed joint gap in {plan:?} tower"
                );
            }
        }
    }

    #[test]
    fn square_tower_corners_stay_crisp_on_every_course() {
        let terrain = SiteTerrain::m0a_promontory();
        for orientation in [0i32, 37, 145] {
            let mut doc = CastleDocument::new();
            let mut tower = RoundTower::new(1, "Square tower", [0, 0], 3_500, 12_000);
            tower.plan = TowerPlan::Square;
            tower.orientation_deg = orientation;
            doc.add_tower(tower.clone());
            let blocks = generate_blocks(&doc, &terrain);

            let o = orientation as f64 * std::f64::consts::PI / 180.0;
            let (oc, os) = (cos(o), sin(o));
            let to_world = |u: f64, v: f64| -> [f64; 2] {
                [
                    tower.centre_mm[0] as f64 + u * oc - v * os,
                    tower.centre_mm[1] as f64 + u * os + v * oc,
                ]
            };
            let to_local = |x: f64, z: f64| -> [f64; 2] {
                let dx = x - tower.centre_mm[0] as f64;
                let dz = z - tower.centre_mm[1] as f64;
                [dx * oc + dz * os, -dx * os + dz * oc]
            };

            let mut courses: std::collections::BTreeMap<i32, (f64, f64, f64, f64, f64)> =
                std::collections::BTreeMap::new();
            for block in &blocks {
                let y0 = block.corners_mm[0][1] as f64;
                let entry = courses.entry(block.course_index).or_insert((
                    f64::MAX,
                    f64::MIN,
                    f64::MAX,
                    f64::MIN,
                    y0,
                ));
                entry.4 = entry.4.max(y0);
                for corner in block.corners_mm.iter().take(4) {
                    let [u, v] = to_local(corner[0] as f64, corner[2] as f64);
                    entry.0 = entry.0.min(u);
                    entry.1 = entry.1.max(u);
                    entry.2 = entry.2.min(v);
                    entry.3 = entry.3.max(v);
                }
            }

            for (course, (u_min, u_max, v_min, v_max, y0)) in courses {
                // Lower courses are legitimately culled where they are buried
                // in the promontory; check the exposed masonry only.
                if y0 < terrain.height_mm(0, 0) as f64 + 1_500.0 {
                    continue;
                }
                let y = y0 + 1.0;
                // The outer corner of a battered square is a single crisp
                // point; probing 5 mm inside each corner must hit masonry.
                for (u, v) in [
                    (u_min + 5.0, v_min + 5.0),
                    (u_max - 5.0, v_min + 5.0),
                    (u_min + 5.0, v_max - 5.0),
                    (u_max - 5.0, v_max - 5.0),
                ] {
                    let world = to_world(u, v);
                    assert!(
                        covered(&blocks, world[0], world[1], y),
                        "orientation {orientation} course {course}: corner ({u:.0}, {v:.0}) is notched"
                    );
                }
            }
        }
    }

    fn polygon_area(poly: &[[f64; 2]]) -> f64 {
        let mut area = 0.0;
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            area += a[0] * b[1] - b[0] * a[1];
        }
        area * 0.5
    }

    fn segment_intersection(p: [f64; 2], q: [f64; 2], a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
        let r = [q[0] - p[0], q[1] - p[1]];
        let s = [b[0] - a[0], b[1] - a[1]];
        let denom = r[0] * s[1] - r[1] * s[0];
        if denom.abs() < 1.0e-9 {
            return p;
        }
        let t = ((a[0] - p[0]) * s[1] - (a[1] - p[1]) * s[0]) / denom;
        [p[0] + r[0] * t, p[1] + r[1] * t]
    }

    /// Sutherland-Hodgman clip of one convex polygon against another.
    fn clip_polygon(subject: &[[f64; 2]], clip: &[[f64; 2]]) -> Vec<[f64; 2]> {
        let mut clip = clip.to_vec();
        if polygon_area(&clip) < 0.0 {
            clip.reverse();
        }
        let mut output = subject.to_vec();
        for i in 0..clip.len() {
            let a = clip[i];
            let b = clip[(i + 1) % clip.len()];
            let input = std::mem::take(&mut output);
            if input.is_empty() {
                break;
            }
            for j in 0..input.len() {
                let p = input[j];
                let q = input[(j + 1) % input.len()];
                let inside = |point: [f64; 2]| {
                    (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0]) >= -1.0e-9
                };
                match (inside(p), inside(q)) {
                    (true, true) => output.push(q),
                    (true, false) => output.push(segment_intersection(p, q, a, b)),
                    (false, true) => {
                        output.push(segment_intersection(p, q, a, b));
                        output.push(q);
                    }
                    (false, false) => {}
                }
            }
        }
        output
    }

    #[test]
    fn dshaped_rear_wall_is_trimmed_to_the_arc_inner_radius() {
        let terrain = SiteTerrain::m0a_promontory();
        for orientation in [0i32, 180, 145] {
            let mut doc = CastleDocument::new();
            let mut tower = RoundTower::new(1, "D tower", [0, 0], 3_500, 12_000);
            tower.thickness_mm = 1_200;
            tower.plan = TowerPlan::DShaped;
            tower.orientation_deg = orientation;
            doc.add_tower(tower.clone());
            let blocks = generate_blocks(&doc, &terrain);

            let o = orientation as f64 * std::f64::consts::PI / 180.0;
            let (oc, os) = (cos(o), sin(o));
            let to_local = |p: [i64; 3]| -> [f64; 2] {
                let dx = p[0] as f64 - tower.centre_mm[0] as f64;
                let dz = p[2] as f64 - tower.centre_mm[1] as f64;
                [dx * oc + dz * os, -dx * os + dz * oc]
            };

            let outer_limit = (tower.radius_mm + tower.plinth_projection_mm + 2) as f64;
            let mut rear_blocks: Vec<&Block> = Vec::new();
            let mut arc_blocks: Vec<&Block> = Vec::new();
            for block in &blocks {
                let local: Vec<[f64; 2]> = block
                    .corners_mm
                    .iter()
                    .map(|corner| to_local(*corner))
                    .collect();
                if block.run_pos_mm >= 1_000_000 {
                    for point in local {
                        assert!(
                            point[0] <= 2.0,
                            "rear block extends past u=0 into arc at u={:.1}",
                            point[0]
                        );
                        assert!(
                            point[1].abs() <= outer_limit,
                            "rear block reaches v={:.1} beyond r_out={outer_limit:.1}",
                            point[1]
                        );
                    }
                    rear_blocks.push(block);
                } else {
                    arc_blocks.push(block);
                }
            }
            assert!(!rear_blocks.is_empty());

            let to_local_plan = |p: [f64; 2]| -> [f64; 2] {
                let dx = p[0] - tower.centre_mm[0] as f64;
                let dz = p[1] - tower.centre_mm[1] as f64;
                [dx * oc + dz * os, -dx * os + dz * oc]
            };

            // No rear stone may share volume with an arc stone on its course.
            for rear in &rear_blocks {
                for arc in &arc_blocks {
                    if rear.course_index != arc.course_index {
                        continue;
                    }
                    let y = (rear.corners_mm[0][1] + rear.corners_mm[4][1]) as f64 / 2.0;
                    let rear_poly: Vec<[f64; 2]> = block_section(rear, y)
                        .iter()
                        .map(|p| to_local_plan(*p))
                        .collect();
                    let arc_poly: Vec<[f64; 2]> = block_section(arc, y)
                        .iter()
                        .map(|p| to_local_plan(*p))
                        .collect();
                    let area = polygon_area(&clip_polygon(&rear_poly, &arc_poly)).abs();
                    assert!(
                        area <= 2.0,
                        "orientation {orientation} course {}: rear/arc overlap {area:.1} mm^2",
                        rear.course_index
                    );
                }
            }
        }
    }

    #[test]
    fn gatehouse_keeps_the_passage_clear_below_the_lintel() {
        let terrain = SiteTerrain::m0a_promontory();
        let mut doc = CastleDocument::new();
        let gatehouse = Gatehouse::new(1, "Gatehouse", [0, 0], 0);
        let ground = terrain.height_mm(0, 0);
        doc.add_gatehouse(gatehouse.clone());
        let blocks = generate_blocks(&doc, &terrain);
        assert_blocks_valid(&blocks, "gatehouse");

        let mut checked = 0;
        let mut x = -3_000;
        while x <= 3_000 {
            let mut y = ground + 100;
            while y < ground + gatehouse.passage_height_mm - 100 {
                for block in &blocks {
                    let (min, max) = block_bounds(block);
                    let inside = (x as f64) > min[0] + 0.5
                        && (x as f64) < max[0] - 0.5
                        && (-100.0f64) > min[2] + 0.5
                        && (100.0f64) < max[2] - 0.5
                        && (y as f64) > min[1] + 0.5
                        && (y as f64) < max[1] - 0.5;
                    assert!(
                        !inside,
                        "gatehouse block obstructs the passage at ({x}, {y})"
                    );
                }
                checked += 1;
                y += 350;
            }
            x += 500;
        }
        assert!(checked > 20);

        // Masonry does span above the passage, so the gatehouse reads as a gate.
        let above = blocks.iter().any(|block| {
            let (min, max) = block_bounds(block);
            min[1] >= (ground + gatehouse.passage_height_mm - 1) as f64
                && max[1] > (ground + gatehouse.passage_height_mm) as f64
        });
        assert!(above, "no masonry bridges the passage");
    }

    #[test]
    fn gatehouse_bridge_is_coursed_ashlar_buried_in_both_drums() {
        let terrain = SiteTerrain::m0a_promontory();
        let mut doc = CastleDocument::new();
        let gatehouse = Gatehouse::new(1, "Gatehouse", [0, 0], 0);
        let ground = terrain.height_mm(0, 0);
        let passage_top = ground + gatehouse.passage_height_mm;
        let passage_half = gatehouse.passage_width_mm / 2;
        let towers = gatehouse.flank_towers();
        let half_span = passage_half + 500 + towers[0].plinth_projection_mm;
        doc.add_gatehouse(gatehouse.clone());
        let blocks = generate_blocks(&doc, &terrain);

        // The bridge starts exactly on the passage crown and spans the whole
        // lateral width, so it is not stranded above a gap.
        let mut bridge: std::collections::BTreeMap<i32, Vec<&Block>> =
            std::collections::BTreeMap::new();
        for block in blocks
            .iter()
            .filter(|b| b.kind == ElementKind::Gatehouse && b.course_index >= 0)
        {
            bridge.entry(block.course_index).or_default().push(block);
        }
        assert!(bridge.len() > 10, "bridge has too few courses");
        let mut courses_with_bond = 0;
        for (course, mut stones) in bridge {
            stones.sort_by_key(|b| b.run_pos_mm);
            assert!(stones[0].corners_mm[0][1] >= passage_top - 1);
            let min_v = stones
                .iter()
                .flat_map(|b| b.corners_mm)
                .map(|c| c[2])
                .min()
                .unwrap();
            let max_v = stones
                .iter()
                .flat_map(|b| b.corners_mm)
                .map(|c| c[2])
                .max()
                .unwrap();
            assert_eq!(min_v, -half_span, "course {course} stops short on the left");
            assert_eq!(max_v, half_span, "course {course} stops short on the right");
            assert!(
                stones.len() >= 4,
                "course {course} is one full-width plank, not ashlar"
            );
            // Neighbouring stones share the exact same joint plane.
            for pair in stones.windows(2) {
                let (left, right) = (pair[0], pair[1]);
                assert_eq!(left.corners_mm[3][2], right.corners_mm[0][2]);
                assert_eq!(left.corners_mm[7][2], right.corners_mm[4][2]);
            }
            // Each end of the course is buried in the drum shell rather than
            // stopping at its surface with a daylight gap.
            let y = (stones[0].corners_mm[0][1] + stones[0].corners_mm[4][1]) / 2;
            for tower in &towers {
                let offset = (tower.centre_mm[1] - gatehouse.centre_mm[1]).abs();
                let inner = junction::tower_inner_radius_at(tower, &terrain, y);
                let outer = junction::tower_outer_radius_at(tower, &terrain, y);
                let reach = offset - half_span;
                assert!(
                    reach > inner && reach < outer,
                    "course {course}: bridge end {reach} mm from drum centre is outside shell {inner}..{outer}"
                );
            }
            courses_with_bond += stones.len();
        }
        assert!(courses_with_bond > 40);

        // Lower side walls fill the band from the passage edge out to the
        // drums, from ground to the passage crown, on both sides.
        let mut side_blocks: Vec<&Block> = blocks
            .iter()
            .filter(|b| b.kind == ElementKind::Gatehouse && b.course_index < 0)
            .collect();
        assert!(!side_blocks.is_empty(), "no passage side walls");
        side_blocks.sort_by_key(|b| (b.course_index, b.run_pos_mm));
        let low = side_blocks
            .iter()
            .map(|b| b.corners_mm[0][1])
            .min()
            .unwrap();
        let high = side_blocks
            .iter()
            .map(|b| b.corners_mm[4][1])
            .max()
            .unwrap();
        assert_eq!(low, ground);
        assert_eq!(high, passage_top);
        for sign in [-1i64, 1] {
            let inner = sign * passage_half;
            let outer = sign * half_span;
            assert!(side_blocks.iter().any(|b| {
                let v0 = b.corners_mm[0][2].min(b.corners_mm[3][2]);
                let v1 = b.corners_mm[0][2].max(b.corners_mm[3][2]);
                v0 == inner.min(outer) && v1 == inner.max(outer)
            }));
        }
    }

    #[test]
    fn a_curtain_near_the_eave_trims_only_the_outer_roof_courses() {
        let terrain = SiteTerrain::for_site("lake_island").expect("curated site");
        let mut doc = CastleDocument::new();
        let mut range = Range::new(30, "Hall", RangeKind::Hall, [0, 0], 0);
        range.length_mm = 18_000;
        range.width_mm = 8_500;
        range.wall_height_mm = 7_500;
        range.roof_pitch_deg = 42;
        doc.add_range(range);

        // The curtain runs parallel to the ridge, its centreline half a metre
        // beyond the eave, so the roof overlaps the wall face by 0.7 m.
        let half_w = 4_250i64;
        let wall_v = half_w + 500;
        let mut wall = BatteredWall::new(
            1,
            "Curtain",
            [-12_000, wall_v],
            [12_000, wall_v],
            10_000,
            2_400,
        );
        wall.batter_mm = 600;
        wall.plinth_height_mm = 700;
        wall.plinth_projection_mm = 250;
        doc.add_wall(wall);

        let blocks = generate_blocks(&doc, &terrain);
        let positive_roof: Vec<_> = blocks
            .iter()
            .filter(|block| {
                block.element_id == 30
                    && block.role == BlockRole::Roof
                    && block.corners_mm.iter().all(|corner| corner[2] >= 0)
            })
            .collect();
        assert!(
            !positive_roof.is_empty(),
            "the south roof slope produced no tiles"
        );

        let min_v = positive_roof
            .iter()
            .flat_map(|block| block.corners_mm.iter().map(|corner| corner[2]))
            .min()
            .unwrap();
        let max_v = positive_roof
            .iter()
            .flat_map(|block| block.corners_mm.iter().map(|corner| corner[2]))
            .max()
            .unwrap();
        assert!(min_v <= 200, "ridge courses are missing: min v {min_v}");
        assert!(
            max_v < half_w - 200,
            "the eave courses next to the curtain were not trimmed: max v {max_v}"
        );
        assert!(
            max_v as f64 >= half_w as f64 * 0.5,
            "more than half the slope was lost: max v {max_v} of {half_w}"
        );

        // No surviving tile may sit inside the wall footprint below its head.
        let ground = terrain
            .height_mm(-12_000, wall_v)
            .max(terrain.height_mm(12_000, wall_v))
            .max(terrain.height_mm(0, wall_v));
        let wall_top = ground + 10_000;
        for corner in positive_roof
            .iter()
            .flat_map(|block| block.corners_mm.iter())
        {
            if (corner[2] - wall_v).abs() <= 1_200 && corner[1] < wall_top {
                panic!("roof tile pokes through the curtain at {corner:?}");
            }
        }
    }

    #[test]
    fn keep_and_range_blocks_are_positive_volume_and_have_a_roof() {
        let terrain = SiteTerrain::for_site("lake_island").expect("curated site");
        let doc = crate::fixtures::m2_baltic_document();
        let blocks = generate_blocks(&doc, &terrain);

        let keep_blocks: Vec<_> = blocks
            .iter()
            .filter(|block| block.element_id == 20)
            .collect();
        let range_blocks: Vec<_> = blocks
            .iter()
            .filter(|block| block.element_id == 30)
            .collect();
        assert!(
            keep_blocks.len() > 500,
            "keep blocks: {}",
            keep_blocks.len()
        );
        assert!(
            range_blocks.len() > 200,
            "range blocks: {}",
            range_blocks.len()
        );
        assert!(range_blocks
            .iter()
            .any(|block| block.role == BlockRole::Roof));

        let mut checked = 0;
        for block in keep_blocks.iter().chain(&range_blocks) {
            for corner in block.corners_mm {
                assert!(corner[0].abs() < 500_000);
                assert!(corner[1].abs() < 200_000);
                assert!(corner[2].abs() < 500_000);
            }
            let volume = signed_volume(&block.corners_mm);
            assert!(
                volume > 1_000.0,
                "{:?} block {} course {} has volume {volume}",
                block.kind,
                block.element_id,
                block.course_index
            );
            checked += 1;
        }
        assert!(checked > 700, "checked only {checked} blocks");
    }

    fn signed_volume(corners: &[[i64; 3]; 8]) -> f64 {
        let point = |index: usize| -> [f64; 3] {
            [
                corners[index][0] as f64,
                corners[index][1] as f64,
                corners[index][2] as f64,
            ]
        };
        let mut volume = 0.0;
        for face in [
            BlockFace::Bottom,
            BlockFace::Top,
            BlockFace::Front,
            BlockFace::Back,
            BlockFace::End0,
            BlockFace::End1,
        ] {
            let ids = Block::face_corners(face);
            let p0 = point(ids[0]);
            let p1 = point(ids[1]);
            let p2 = point(ids[2]);
            let p3 = point(ids[3]);
            volume += dot(p0, cross(p1, p2));
            volume += dot(p0, cross(p2, p3));
        }
        volume / 6.0
    }

    fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn every_block_caps_its_course_with_a_top_face() {
        let cases = [
            (SiteTerrain::m0a_promontory(), m0a_showcase_document()),
            (
                SiteTerrain::for_site("coastal_headland").expect("curated site"),
                m1_edwardian_document(),
            ),
        ];
        for (terrain, doc) in cases {
            let blocks = generate_blocks(&doc, &terrain);
            assert!(!blocks.is_empty());
            // Construction playback hides the course above while the active
            // course rises; an uncapped course would read as a hollow shell.
            for block in &blocks {
                assert!(
                    block.expose[BlockFace::Top as usize],
                    "{:?} block {} course {} has no top cap",
                    block.kind,
                    block.element_id,
                    block.course_index
                );
            }
        }
    }
}
