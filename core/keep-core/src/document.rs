//! The Castle Document: the single source of truth for castle architecture.
//!
//! M0a only needed two semantic elements — a battered curtain wall and a round
//! tower — but they already exercise the important rules: stable ids, sorted
//! storage, integer-millimetre inputs, and a zero-dependency layout hash that
//! can be compared across machines. M0b adds *openings* (attached features such
//! as splayed arrow loops) that are anchored to a wall by a relative position
//! along its run rather than by absolute coordinates.

use std::collections::BTreeMap;

use crate::fnv1a_64;

/// Fixed site/design seed used by the M0a fixtures.
pub const CASTLE_SEED: u64 = 0x4b45_4550_5752_4947;

pub type ElementId = u64;

/// Broad element classification, used to choose appropriate meshing rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ElementKind {
    BatteredWall,
    RoundTower,
}

/// A straight curtain-wall run with vertical batter and a projecting plinth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatteredWall {
    pub id: ElementId,
    pub name: String,
    /// Start point on the plan, `[x, z]` millimetres.
    pub start_mm: [i64; 2],
    /// End point on the plan, `[x, z]` millimetres.
    pub end_mm: [i64; 2],
    pub height_mm: i64,
    pub thickness_mm: i64,
    /// Total horizontal inset of each face over the full height.
    pub batter_mm: i64,
    pub plinth_height_mm: i64,
    pub plinth_projection_mm: i64,
    pub course_height_mm: i64,
    pub seed: u64,
}

impl BatteredWall {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ElementId,
        name: impl Into<String>,
        start_mm: [i64; 2],
        end_mm: [i64; 2],
        height_mm: i64,
        thickness_mm: i64,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            start_mm,
            end_mm,
            height_mm,
            thickness_mm,
            batter_mm: 900,
            plinth_height_mm: 900,
            plinth_projection_mm: 300,
            course_height_mm: 450,
            seed: CASTLE_SEED ^ id.rotate_left(17),
        }
    }

    pub fn length_mm(&self) -> i64 {
        let dx = self.end_mm[0] as i128 - self.start_mm[0] as i128;
        let dz = self.end_mm[1] as i128 - self.start_mm[1] as i128;
        // Saturating i128 arithmetic: even coordinates spanning the full i64
        // range cannot overflow or panic here.
        let squared = dx.saturating_mul(dx).saturating_add(dz.saturating_mul(dz));
        let length = libm::sqrt(squared as f64);
        if !length.is_finite() || length >= i64::MAX as f64 {
            i64::MAX
        } else {
            libm::round(length) as i64
        }
    }
}

/// A cylindrical tower with batter, a projecting plinth and a wall thickness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoundTower {
    pub id: ElementId,
    pub name: String,
    /// Plan centre, `[x, z]` millimetres.
    pub centre_mm: [i64; 2],
    pub radius_mm: i64,
    pub height_mm: i64,
    pub thickness_mm: i64,
    pub batter_mm: i64,
    pub plinth_height_mm: i64,
    pub plinth_projection_mm: i64,
    pub course_height_mm: i64,
    pub seed: u64,
}

impl RoundTower {
    pub fn new(
        id: ElementId,
        name: impl Into<String>,
        centre_mm: [i64; 2],
        radius_mm: i64,
        height_mm: i64,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            centre_mm,
            radius_mm,
            height_mm,
            thickness_mm: (radius_mm * 34 / 100).clamp(900, 1_600),
            batter_mm: 500,
            plinth_height_mm: 1_200,
            plinth_projection_mm: 350,
            course_height_mm: 450,
            seed: CASTLE_SEED ^ id.rotate_left(29),
        }
    }
}

/// Classification of an opening cut through a wall.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OpeningKind {
    /// Narrow outer slit opening into a wide, splayed inner embrasure.
    ArrowLoop,
}

/// A void cut through a [`BatteredWall`], anchored relatively so it survives
/// edits to the wall's plan geometry.
///
/// The opening is positioned by `t_permille` along the wall run (`0` at the
/// start, `1000` at the end). The sill height is measured above the ground at
/// that point, not above the wall foundation, so lowering the site keeps the
/// loop at a sensible height above the earth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opening {
    pub id: ElementId,
    pub wall_id: ElementId,
    pub kind: OpeningKind,
    pub name: String,
    /// Position along the wall run, `0..=1000`.
    pub t_permille: i64,
    /// Height of the void bottom above the ground at the opening, in mm.
    pub sill_height_mm: i64,
    /// Height of the void, in mm.
    pub height_mm: i64,
    /// Width of the slit at the outer wall face, in mm.
    pub outer_slit_mm: i64,
    /// Width of the splayed embrasure at the inner wall face, in mm.
    pub inner_splay_width_mm: i64,
    /// Depth of the splay through the wall, in mm. Clamped to the wall
    /// thickness when the void is built.
    pub splay_depth_mm: i64,
}

impl Opening {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ElementId,
        wall_id: ElementId,
        name: impl Into<String>,
        kind: OpeningKind,
        t_permille: i64,
        sill_height_mm: i64,
        height_mm: i64,
        outer_slit_mm: i64,
        inner_splay_width_mm: i64,
        splay_depth_mm: i64,
    ) -> Self {
        Self {
            id,
            wall_id,
            kind,
            name: name.into(),
            t_permille: t_permille.clamp(0, 1000),
            sill_height_mm,
            height_mm,
            outer_slit_mm: outer_slit_mm.max(1),
            inner_splay_width_mm: inner_splay_width_mm.max(outer_slit_mm.max(1)),
            splay_depth_mm,
        }
    }

    /// Convenience constructor for the M0b splayed arrow loop.
    pub fn arrow_loop(
        id: ElementId,
        wall_id: ElementId,
        name: impl Into<String>,
        t_permille: i64,
        sill_height_mm: i64,
    ) -> Self {
        Self::new(
            id,
            wall_id,
            name,
            OpeningKind::ArrowLoop,
            t_permille,
            sill_height_mm,
            1_500,
            140,
            1_100,
            2_500,
        )
    }

    /// Plan point of the opening on its wall's centreline, `[x, z]` mm.
    pub fn plan_point_mm(&self, wall: &BatteredWall) -> [i64; 2] {
        let t = self.t_permille as f64 / 1000.0;
        let x = wall.start_mm[0] as f64 + (wall.end_mm[0] - wall.start_mm[0]) as f64 * t;
        let z = wall.start_mm[1] as f64 + (wall.end_mm[1] - wall.start_mm[1]) as f64 * t;
        [libm::round(x) as i64, libm::round(z) as i64]
    }

    /// Plan point of the opening, looked up through the document. Returns
    /// `None` when the wall is missing or is not a [`BatteredWall`].
    pub fn plan_point(&self, document: &CastleDocument) -> Option<[i64; 2]> {
        match document.get(self.wall_id) {
            Some(Element::BatteredWall(wall)) => Some(self.plan_point_mm(wall)),
            _ => None,
        }
    }
}

/// One architectural element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Element {
    BatteredWall(BatteredWall),
    RoundTower(RoundTower),
}

impl Element {
    pub fn id(&self) -> ElementId {
        match self {
            Self::BatteredWall(w) => w.id,
            Self::RoundTower(t) => t.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::BatteredWall(w) => &w.name,
            Self::RoundTower(t) => &t.name,
        }
    }

    pub fn kind(&self) -> ElementKind {
        match self {
            Self::BatteredWall(_) => ElementKind::BatteredWall,
            Self::RoundTower(_) => ElementKind::RoundTower,
        }
    }
}

/// Canonical castle document. Elements are keyed and iterated in stable id
/// order so layout never depends on insertion order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CastleDocument {
    elements: BTreeMap<ElementId, Element>,
    openings: BTreeMap<ElementId, Opening>,
}

impl CastleDocument {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an element. Ids are unique; replacing an existing id is allowed
    /// and keeps deterministic ordering.
    pub fn insert(&mut self, element: Element) {
        self.elements.insert(element.id(), element);
    }

    pub fn add_wall(&mut self, wall: BatteredWall) -> ElementId {
        let id = wall.id;
        self.insert(Element::BatteredWall(wall));
        id
    }

    pub fn add_tower(&mut self, tower: RoundTower) -> ElementId {
        let id = tower.id;
        self.insert(Element::RoundTower(tower));
        id
    }

    /// Insert an opening. Ids are unique across openings; replacing an
    /// existing id is allowed.
    pub fn insert_opening(&mut self, opening: Opening) {
        self.openings.insert(opening.id, opening);
    }

    pub fn add_opening(&mut self, opening: Opening) -> ElementId {
        let id = opening.id;
        self.insert_opening(opening);
        id
    }

    pub fn opening(&self, id: ElementId) -> Option<&Opening> {
        self.openings.get(&id)
    }

    /// All openings in stable id order.
    pub fn openings(&self) -> impl Iterator<Item = &Opening> {
        self.openings.values()
    }

    /// Openings attached to `wall_id`, in stable id order.
    pub fn openings_for_wall(&self, wall_id: ElementId) -> impl Iterator<Item = &Opening> {
        self.openings.values().filter(move |o| o.wall_id == wall_id)
    }

    pub fn opening_count(&self) -> usize {
        self.openings.len()
    }

    pub fn get(&self, id: ElementId) -> Option<&Element> {
        self.elements.get(&id)
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.elements.values()
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Deterministic 64-bit document hash over the canonical semantic contents.
    /// This is the "document hash" half of the co-play divergence check: a
    /// mismatch means a missed or corrupted command. It deliberately excludes
    /// the display `name` and `GENERATOR_VERSION`; the generated-geometry
    /// `layout_hash` in [`crate::mesh::build_scene`] covers generator changes.
    pub fn document_hash(&self) -> u64 {
        let mut bytes =
            Vec::with_capacity(64 + self.elements.len() * 128 + self.openings.len() * 96);
        bytes.extend_from_slice(b"keepwright-document-v2");
        bytes.extend_from_slice(&(self.elements.len() as u64).to_le_bytes());
        for element in self.elements.values() {
            match element {
                Element::BatteredWall(w) => {
                    bytes.push(1);
                    bytes.extend_from_slice(&w.id.to_le_bytes());
                    bytes.extend_from_slice(&w.start_mm[0].to_le_bytes());
                    bytes.extend_from_slice(&w.start_mm[1].to_le_bytes());
                    bytes.extend_from_slice(&w.end_mm[0].to_le_bytes());
                    bytes.extend_from_slice(&w.end_mm[1].to_le_bytes());
                    bytes.extend_from_slice(&w.height_mm.to_le_bytes());
                    bytes.extend_from_slice(&w.thickness_mm.to_le_bytes());
                    bytes.extend_from_slice(&w.batter_mm.to_le_bytes());
                    bytes.extend_from_slice(&w.plinth_height_mm.to_le_bytes());
                    bytes.extend_from_slice(&w.plinth_projection_mm.to_le_bytes());
                    bytes.extend_from_slice(&w.course_height_mm.to_le_bytes());
                    bytes.extend_from_slice(&w.seed.to_le_bytes());
                }
                Element::RoundTower(t) => {
                    bytes.push(2);
                    bytes.extend_from_slice(&t.id.to_le_bytes());
                    bytes.extend_from_slice(&t.centre_mm[0].to_le_bytes());
                    bytes.extend_from_slice(&t.centre_mm[1].to_le_bytes());
                    bytes.extend_from_slice(&t.radius_mm.to_le_bytes());
                    bytes.extend_from_slice(&t.height_mm.to_le_bytes());
                    bytes.extend_from_slice(&t.thickness_mm.to_le_bytes());
                    bytes.extend_from_slice(&t.batter_mm.to_le_bytes());
                    bytes.extend_from_slice(&t.plinth_height_mm.to_le_bytes());
                    bytes.extend_from_slice(&t.plinth_projection_mm.to_le_bytes());
                    bytes.extend_from_slice(&t.course_height_mm.to_le_bytes());
                    bytes.extend_from_slice(&t.seed.to_le_bytes());
                }
            }
        }
        bytes.extend_from_slice(&(self.openings.len() as u64).to_le_bytes());
        for opening in self.openings.values() {
            bytes.push(opening.kind as u8);
            bytes.extend_from_slice(&opening.id.to_le_bytes());
            bytes.extend_from_slice(&opening.wall_id.to_le_bytes());
            bytes.extend_from_slice(&opening.t_permille.to_le_bytes());
            bytes.extend_from_slice(&opening.sill_height_mm.to_le_bytes());
            bytes.extend_from_slice(&opening.height_mm.to_le_bytes());
            bytes.extend_from_slice(&opening.outer_slit_mm.to_le_bytes());
            bytes.extend_from_slice(&opening.inner_splay_width_mm.to_le_bytes());
            bytes.extend_from_slice(&opening.splay_depth_mm.to_le_bytes());
        }
        fnv1a_64(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_sorted_and_hash_is_order_independent() {
        let mut a = CastleDocument::new();
        a.add_wall(BatteredWall::new(
            20,
            "b",
            [0, 0],
            [10_000, 0],
            9_000,
            2_400,
        ));
        a.add_wall(BatteredWall::new(
            10,
            "a",
            [0, 5_000],
            [10_000, 5_000],
            8_000,
            2_000,
        ));

        let mut b = CastleDocument::new();
        b.add_wall(BatteredWall::new(
            10,
            "a",
            [0, 5_000],
            [10_000, 5_000],
            8_000,
            2_000,
        ));
        b.add_wall(BatteredWall::new(
            20,
            "b",
            [0, 0],
            [10_000, 0],
            9_000,
            2_400,
        ));

        assert_eq!(a.document_hash(), b.document_hash());
        let ids: Vec<_> = a.elements().map(Element::id).collect();
        assert_eq!(ids, vec![10, 20]);
    }

    #[test]
    fn changing_a_quantised_field_changes_the_hash() {
        let mut doc = CastleDocument::new();
        doc.add_wall(BatteredWall::new(
            1,
            "wall",
            [0, 0],
            [10_000, 0],
            9_000,
            2_400,
        ));
        let before = doc.document_hash();
        let mut wall = match doc.get(1).unwrap() {
            Element::BatteredWall(w) => w.clone(),
            _ => unreachable!(),
        };
        wall.height_mm += 1;
        doc.add_wall(wall);
        assert_ne!(before, doc.document_hash());
    }

    #[test]
    fn display_names_do_not_change_the_document_hash() {
        let mut a = CastleDocument::new();
        a.add_wall(BatteredWall::new(
            1,
            "north",
            [0, 0],
            [10_000, 0],
            9_000,
            2_400,
        ));
        let mut b = CastleDocument::new();
        b.add_wall(BatteredWall::new(
            1,
            "south",
            [0, 0],
            [10_000, 0],
            9_000,
            2_400,
        ));
        assert_eq!(a.document_hash(), b.document_hash());
    }

    #[test]
    fn extreme_coordinates_do_not_overflow_length() {
        let wall = BatteredWall::new(
            1,
            "huge",
            [i64::MIN + 1_000, i64::MIN + 1_000],
            [i64::MAX - 1_000, i64::MAX - 1_000],
            9_000,
            2_400,
        );
        let length = wall.length_mm();
        assert!(length > 0);
        assert_eq!(
            length,
            BatteredWall::new(2, "same", wall.start_mm, wall.end_mm, 9_000, 2_400).length_mm()
        );
    }

    #[test]
    fn openings_change_the_document_hash_and_resolve_plan_points() {
        let mut doc = CastleDocument::new();
        doc.add_wall(BatteredWall::new(
            1,
            "wall",
            [0, 0],
            [10_000, 0],
            9_000,
            2_400,
        ));
        let wall = match doc.get(1).unwrap() {
            Element::BatteredWall(w) => w.clone(),
            _ => unreachable!(),
        };
        let before = doc.document_hash();
        let opening = Opening::arrow_loop(10, 1, "loop", 250, 1_500);
        assert_eq!(opening.plan_point_mm(&wall), [2_500, 0]);
        doc.add_opening(opening);
        assert_ne!(before, doc.document_hash());
        assert_eq!(doc.opening_count(), 1);
        assert_eq!(doc.openings_for_wall(1).count(), 1);
        assert_eq!(doc.openings_for_wall(2).count(), 0);

        // Openings are hashed in sorted id order, independent of insertion.
        let mut other = CastleDocument::new();
        other.add_wall(wall);
        other.add_opening(Opening::arrow_loop(11, 1, "b", 750, 1_700));
        other.add_opening(Opening::arrow_loop(10, 1, "a", 250, 1_500));
        let mut sorted = CastleDocument::new();
        sorted.add_wall(match other.get(1).unwrap() {
            Element::BatteredWall(w) => w.clone(),
            _ => unreachable!(),
        });
        sorted.add_opening(Opening::arrow_loop(10, 1, "a", 250, 1_500));
        sorted.add_opening(Opening::arrow_loop(11, 1, "b", 750, 1_700));
        assert_eq!(other.document_hash(), sorted.document_hash());
    }
}
