//! Deterministic dollhouse interior props.
//!
//! The document remains the single source of truth: props are *derived* from
//! the semantic ranges and keeps each time the interior is queried. Placement
//! uses integer millimetres, stable element order and `libm` trigonometry, so
//! the same document always produces the same furniture layout.
//!
//! Every prop is inset from the element's inner wall face by
//! `thickness_mm + 600` mm, keeping furniture clear of the masonry and of the
//! cutaway plane.

use libm::{cos, round, sin};
use serde::{Deserialize, Serialize};

use crate::document::{CastleDocument, ElementId, Keep, Range, RangeKind};
use crate::terrain::SiteTerrain;

/// Which horizontal or vertical plane cuts the dollhouse open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionAxis {
    X,
    Z,
}

/// Cutaway section plane for the plan renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SectionPlane {
    Off,
    Horizontal { y_mm: i64 },
    Vertical { axis: SectionAxis, pos_mm: i64 },
}

/// Kind of low-poly interior prop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropKind {
    Table,
    Bench,
    Bed,
    Altar,
    Cauldron,
    Barrel,
    Chest,
}

impl PropKind {
    /// Stable snake_case name, also used as the plan marker label.
    pub fn kind_name(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Bench => "bench",
            Self::Bed => "bed",
            Self::Altar => "altar",
            Self::Cauldron => "cauldron",
            Self::Barrel => "barrel",
            Self::Chest => "chest",
        }
    }
}

/// One piece of furniture placed inside a roofed element.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteriorProp {
    pub kind: PropKind,
    pub room_type: String,
    /// World position `[x, y, z]` in millimetres.
    pub pos_mm: [i64; 3],
    /// Yaw clockwise from +X, in degrees.
    pub yaw_deg: i32,
    pub element_id: ElementId,
}

/// Extra clearance from the inner wall face, in millimetres.
const PROP_CLEARANCE_MM: i64 = 600;
/// Extra margin a prop's bounding box must keep from the interior limit, in mm.
const PROP_MARGIN_MM: i64 = 150;
/// Height of the upper solar floor above the keep's ground floor, in mm.
const SOLAR_FLOOR_MM: i64 = 4_500;

/// Exact wall thickness used for a domestic range. Mirrors
/// `masonry::range_wall_thickness_mm` so interior clearance matches the real
/// masonry instead of a fixed guess (M3.1 H3).
fn range_wall_thickness_mm(range: &Range) -> i64 {
    (range.wall_height_mm / 8).clamp(600, 1_200)
}

/// Plan half-extents of each prop's low-poly mesh, `[along local X, along local
/// Z]` in millimetres. These mirror the box/cylinder sizes built in
/// `game/scripts/interior_props.gd` so containment checks use the real footprint
/// rather than a point (M3.1 M5).
fn prop_half_extents_mm(kind: PropKind) -> (i64, i64) {
    match kind {
        // 1.80 m table top with legs out to ±0.69 m.
        PropKind::Table => (900, 425),
        // 1.70 m bench top with legs at ±0.65 m.
        PropKind::Bench => (850, 180),
        // 1.90 × 1.05 m bed base.
        PropKind::Bed => (950, 525),
        // 1.84 × 0.80 m altar mensa.
        PropKind::Altar => (920, 400),
        // 0.48 m radius cauldron rim.
        PropKind::Cauldron => (480, 480),
        // 0.335 m barrel hoop radius.
        PropKind::Barrel => (335, 335),
        // 1.00 × 0.60 m chest lid.
        PropKind::Chest => (500, 300),
    }
}

/// True when a prop centred at local `(u, v)` fits wholly inside the interior
/// with its real half-extents plus the clearance margin.
fn prop_fits(kind: PropKind, u: i64, v: i64, interior_half: [i64; 2]) -> bool {
    let (half_u, half_v) = prop_half_extents_mm(kind);
    u.abs() + half_u + PROP_MARGIN_MM <= interior_half[0]
        && v.abs() + half_v + PROP_MARGIN_MM <= interior_half[1]
}

/// Build every interior prop for every range and keep, in stable id order.
pub fn build_interior_props(doc: &CastleDocument, terrain: &SiteTerrain) -> Vec<InteriorProp> {
    let ditches: Vec<&crate::document::Ditch> = doc.ditches().collect();
    let sculpts: Vec<&crate::document::TerrainSculpt> = doc.terrain_sculpts().collect();
    let effective_terrain = terrain.with_earthworks(&ditches, &sculpts);

    let mut props = Vec::new();
    for element in doc.elements() {
        match element {
            crate::document::Element::Range(range) => {
                let ground = effective_terrain.height_mm(range.centre_mm[0], range.centre_mm[1]);
                match range.kind {
                    RangeKind::Hall => build_hall(range, ground, &mut props),
                    RangeKind::Chapel => build_chapel(range, ground, &mut props),
                    RangeKind::Kitchen => build_kitchen(range, ground, &mut props),
                }
            }
            crate::document::Element::Keep(keep) => {
                let ground = effective_terrain.height_mm(keep.centre_mm[0], keep.centre_mm[1]);
                build_keep(keep, ground, &mut props);
            }
            _ => {}
        }
    }
    props
}

/// Interior half extents of a range, in `[along-ridge, across-ridge]` mm.
fn range_interior_half_extents(range: &Range) -> [i64; 2] {
    let inset = range_wall_thickness_mm(range) + PROP_CLEARANCE_MM;
    [range.length_mm / 2 - inset, range.width_mm / 2 - inset]
}

/// Interior half extents of a keep, in `[width, depth]` mm.
fn keep_interior_half_extents(keep: &Keep) -> [i64; 2] {
    let inset = keep.wall_thickness_mm + PROP_CLEARANCE_MM;
    [keep.width_mm / 2 - inset, keep.depth_mm / 2 - inset]
}

/// Rotate a local `(u, v)` offset into world plan millimetres.
fn local_to_world(centre_mm: [i64; 2], orientation_deg: i32, u: i64, v: i64) -> [i64; 2] {
    let angle = orientation_deg as f64 * std::f64::consts::PI / 180.0;
    let c = cos(angle);
    let s = sin(angle);
    let x = centre_mm[0] as f64 + u as f64 * c - v as f64 * s;
    let z = centre_mm[1] as f64 + u as f64 * s + v as f64 * c;
    [round(x) as i64, round(z) as i64]
}

#[allow(clippy::too_many_arguments)]
fn prop(
    kind: PropKind,
    room_type: &str,
    element_id: ElementId,
    centre_mm: [i64; 2],
    orientation_deg: i32,
    u: i64,
    v: i64,
    y_mm: i64,
) -> InteriorProp {
    let plan = local_to_world(centre_mm, orientation_deg, u, v);
    InteriorProp {
        kind,
        room_type: room_type.to_string(),
        pos_mm: [plan[0], y_mm, plan[1]],
        yaw_deg: orientation_deg,
        element_id,
    }
}

/// Place a prop only when its full footprint fits inside the room. Small rooms
/// thereby drop furniture cleanly instead of letting it intersect the walls or
/// spill outside the cutaway (M3.1 H3/M5).
#[allow(clippy::too_many_arguments)]
fn place_prop(
    props: &mut Vec<InteriorProp>,
    kind: PropKind,
    room_type: &str,
    element_id: ElementId,
    centre_mm: [i64; 2],
    orientation_deg: i32,
    u: i64,
    v: i64,
    y_mm: i64,
    interior_half: [i64; 2],
) {
    if !prop_fits(kind, u, v, interior_half) {
        return;
    }
    props.push(prop(
        kind,
        room_type,
        element_id,
        centre_mm,
        orientation_deg,
        u,
        v,
        y_mm,
    ));
}

fn build_hall(range: &Range, ground: i64, props: &mut Vec<InteriorProp>) {
    let half = range_interior_half_extents(range);
    let room = "great_hall";
    let c = range.centre_mm;
    let o = range.orientation_deg;

    // Two long tables down the hall, each with a flanking bench.
    for (index, u) in [-half[0] / 3, half[0] / 3].into_iter().enumerate() {
        place_prop(
            props,
            PropKind::Table,
            room,
            range.id,
            c,
            o,
            u,
            0,
            ground,
            half,
        );
        let bench_offset = (half[1] / 5).max(600);
        for sign in [-1i64, 1] {
            place_prop(
                props,
                PropKind::Bench,
                room,
                range.id,
                c,
                o,
                u,
                sign * bench_offset,
                ground,
                half,
            );
        }
        // The high table sits at the dais end of the hall.
        if index == 1 {
            place_prop(
                props,
                PropKind::Table,
                room,
                range.id,
                c,
                o,
                half[0] - 500,
                0,
                ground,
                half,
            );
        }
    }

    // Cauldron near the hearth end, against one side wall.
    place_prop(
        props,
        PropKind::Cauldron,
        room,
        range.id,
        c,
        o,
        -(half[0] - 500),
        half[1] / 2,
        ground,
        half,
    );
}

fn build_chapel(range: &Range, ground: i64, props: &mut Vec<InteriorProp>) {
    let half = range_interior_half_extents(range);
    let room = "chapel";
    let c = range.centre_mm;
    let o = range.orientation_deg;

    // Altar at the east/far end of the chapel, inset by its own footprint so it
    // always clears the end wall.
    let (altar_half_u, _) = prop_half_extents_mm(PropKind::Altar);
    place_prop(
        props,
        PropKind::Altar,
        room,
        range.id,
        c,
        o,
        half[0] - altar_half_u - 400,
        0,
        ground,
        half,
    );

    // Rows of benches facing the altar.
    let rows = 3;
    for index in 0..rows {
        let t = index as i64;
        let u = -(half[0] - 1_200) + t * (half[0] / 3).max(600);
        for sign in [-1i64, 1] {
            let v = sign * (half[1] / 3).max(500);
            place_prop(
                props,
                PropKind::Bench,
                room,
                range.id,
                c,
                o,
                u,
                v,
                ground,
                half,
            );
        }
    }
}

fn build_kitchen(range: &Range, ground: i64, props: &mut Vec<InteriorProp>) {
    let half = range_interior_half_extents(range);
    let room = "kitchen";
    let c = range.centre_mm;
    let o = range.orientation_deg;

    place_prop(
        props,
        PropKind::Cauldron,
        room,
        range.id,
        c,
        o,
        half[0] / 2,
        0,
        ground,
        half,
    );
    place_prop(
        props,
        PropKind::Table,
        room,
        range.id,
        c,
        o,
        0,
        0,
        ground,
        half,
    );
    for sign in [-1i64, 1] {
        place_prop(
            props,
            PropKind::Barrel,
            room,
            range.id,
            c,
            o,
            -half[0] / 2,
            sign * (half[1] / 3).max(400),
            ground,
            half,
        );
    }
}

fn build_keep(keep: &Keep, ground: i64, props: &mut Vec<InteriorProp>) {
    let half = keep_interior_half_extents(keep);
    let c = keep.centre_mm;
    let o = keep.orientation_deg;

    // Ground-floor undercroft: barrels and a strong chest.
    for (index, (u, v)) in [
        (-half[0] / 2, -half[1] / 3),
        (-half[0] / 4, -half[1] / 3),
        (half[0] / 3, half[1] / 3),
    ]
    .into_iter()
    .enumerate()
    {
        let kind = if index == 2 {
            PropKind::Chest
        } else {
            PropKind::Barrel
        };
        place_prop(props, kind, "undercroft", keep.id, c, o, u, v, ground, half);
    }

    // Upper solar floor: only when a second storey genuinely fits above the
    // ground floor, otherwise the furniture would float in mid-air (M3.1 H3).
    if keep.height_mm < SOLAR_FLOOR_MM + 2_500 {
        return;
    }
    let solar_y = ground + SOLAR_FLOOR_MM;
    place_prop(
        props,
        PropKind::Bed,
        "solar",
        keep.id,
        c,
        o,
        -half[0] / 3,
        0,
        solar_y,
        half,
    );
    place_prop(
        props,
        PropKind::Table,
        "solar",
        keep.id,
        c,
        o,
        half[0] / 3,
        0,
        solar_y,
        half,
    );
    place_prop(
        props,
        PropKind::Chest,
        "solar",
        keep.id,
        c,
        o,
        half[0] / 3,
        half[1] / 2,
        solar_y,
        half,
    );
}

/// World-space plan points of an element's outer footprint. Used by callers
/// that need to reason about containment without duplicating rotation maths.
pub fn rotated_rect_corners(
    centre_mm: [i64; 2],
    orientation_deg: i32,
    half_u_mm: i64,
    half_v_mm: i64,
) -> [[i64; 2]; 4] {
    let local = [
        (-half_u_mm, -half_v_mm),
        (half_u_mm, -half_v_mm),
        (half_u_mm, half_v_mm),
        (-half_u_mm, half_v_mm),
    ];
    let mut corners = [[0i64; 2]; 4];
    for (index, (u, v)) in local.into_iter().enumerate() {
        corners[index] = local_to_world(centre_mm, orientation_deg, u, v);
    }
    corners
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{CastleDocument, Element, Range, RangeKind};
    use crate::fixtures::m2_baltic_document;

    fn world_to_local(centre: [i64; 2], orientation_deg: i32, x: i64, z: i64) -> (f64, f64) {
        let angle = orientation_deg as f64 * std::f64::consts::PI / 180.0;
        let (c, s) = (cos(angle), sin(angle));
        let dx = (x - centre[0]) as f64;
        let dz = (z - centre[1]) as f64;
        (dx * c + dz * s, -dx * s + dz * c)
    }

    /// Check every emitted prop's full footprint (centre ± half extents) plus
    /// the margin against the room's true inner face.
    fn assert_props_fit(document: &CastleDocument, terrain: &SiteTerrain) -> Vec<InteriorProp> {
        let props = build_interior_props(document, terrain);
        for prop in &props {
            let (inner_half, label) = match document.get(prop.element_id) {
                Some(Element::Range(range)) => (
                    [
                        range.length_mm / 2 - range_wall_thickness_mm(range) - PROP_CLEARANCE_MM,
                        range.width_mm / 2 - range_wall_thickness_mm(range) - PROP_CLEARANCE_MM,
                    ],
                    format!("range {}", range.id),
                ),
                Some(Element::Keep(keep)) => (
                    [
                        keep.width_mm / 2 - keep.wall_thickness_mm - PROP_CLEARANCE_MM,
                        keep.depth_mm / 2 - keep.wall_thickness_mm - PROP_CLEARANCE_MM,
                    ],
                    format!("keep {}", keep.id),
                ),
                other => panic!("prop attached to non-interior element: {other:?}"),
            };
            let (centre, orientation) = match document.get(prop.element_id) {
                Some(Element::Range(range)) => (range.centre_mm, range.orientation_deg),
                Some(Element::Keep(keep)) => (keep.centre_mm, keep.orientation_deg),
                _ => unreachable!("checked above"),
            };
            let (u, v) = world_to_local(centre, orientation, prop.pos_mm[0], prop.pos_mm[2]);
            let (half_u, half_v) = prop_half_extents_mm(prop.kind);
            assert!(
                u.abs() + (half_u + PROP_MARGIN_MM) as f64 <= inner_half[0] as f64 + 1.0,
                "{} on {label} crosses the inner wall in u: |{u}| + {half_u} + {PROP_MARGIN_MM} > {}",
                prop.kind.kind_name(),
                inner_half[0]
            );
            assert!(
                v.abs() + (half_v + PROP_MARGIN_MM) as f64 <= inner_half[1] as f64 + 1.0,
                "{} on {label} crosses the inner wall in v: |{v}| + {half_v} + {PROP_MARGIN_MM} > {}",
                prop.kind.kind_name(),
                inner_half[1]
            );
        }
        props
    }

    #[test]
    fn m2_baltic_document_places_every_prop_kind_inside_its_footprint() {
        let document = m2_baltic_document();
        let terrain = SiteTerrain::for_site("lake_island").unwrap();
        let props = assert_props_fit(&document, &terrain);
        assert!(!props.is_empty());

        let mut kinds = std::collections::BTreeSet::new();
        let mut rooms = std::collections::BTreeSet::new();
        for prop in &props {
            kinds.insert(prop.kind.kind_name());
            rooms.insert(prop.room_type.as_str());
        }
        for kind in [
            "table", "bench", "bed", "altar", "cauldron", "barrel", "chest",
        ] {
            assert!(kinds.contains(kind), "missing prop kind {kind}");
        }
        for room in ["great_hall", "chapel", "kitchen", "undercroft", "solar"] {
            assert!(rooms.contains(room), "missing room type {room}");
        }
    }

    #[test]
    fn minimum_size_chapel_skips_props_that_cannot_fit() {
        let mut document = CastleDocument::new();
        let mut chapel = Range::new(1, "Tiny chapel", RangeKind::Chapel, [0, 0], 30);
        chapel.length_mm = 4_000;
        chapel.width_mm = 3_000;
        chapel.wall_height_mm = 4_500;
        document.add_range(chapel);

        let terrain = SiteTerrain::for_site("lake_island").unwrap();
        let props = assert_props_fit(&document, &terrain);
        // The room is too small for any full-size prop, so all of them are
        // skipped rather than poking through the walls.
        assert!(
            props.is_empty(),
            "a 4x3 m chapel should not fit any prop, got {props:?}"
        );
    }

    #[test]
    fn interior_props_are_deterministic_and_ordered_by_element_id() {
        let document = m2_baltic_document();
        let terrain = SiteTerrain::for_site("lake_island").unwrap();
        let a = build_interior_props(&document, &terrain);
        let b = build_interior_props(&document, &terrain);
        assert_eq!(a, b);
        let ids: Vec<ElementId> = a.iter().map(|prop| prop.element_id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted);
    }
}
