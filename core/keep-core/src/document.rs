//! The Castle Document: the single source of truth for castle architecture.
//!
//! M0a only needs two semantic elements — a battered curtain wall and a round
//! tower — but they already exercise the important rules: stable ids, sorted
//! storage, integer-millimetre inputs, and a zero-dependency layout hash that
//! can be compared across machines.

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
        let dx = self.end_mm[0] - self.start_mm[0];
        let dz = self.end_mm[1] - self.start_mm[1];
        libm::sqrt((dx * dx + dz * dz) as f64).round() as i64
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
        let mut bytes = Vec::with_capacity(64 + self.elements.len() * 128);
        bytes.extend_from_slice(b"keepwright-document-v1");
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
}
