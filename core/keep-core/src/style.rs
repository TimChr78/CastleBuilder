//! Data-driven architectural style packs.
//!
//! A style pack is plain JSON shipped under `styles/`, so it can be reviewed,
//! diffed and modded without recompiling the core. The pack carries the rules a
//! master mason would recognise: course height, wall thickness, slenderness,
//! batter, plinth and the flanking distance between towers, each backed by a
//! short source note (architecture sections 5.1, 6 and 10).

use serde::{Deserialize, Serialize};

/// The Edwardian Welsh stone style used by the M1 vertical slice.
pub const EDWARDIAN_WELSH_JSON: &str = include_str!("../../../styles/edwardian_welsh.json");

/// One curated style pack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StylePack {
    pub id: String,
    pub name: String,
    pub period: String,
    pub region: String,
    /// Nominal ashlar course height, in millimetres.
    pub course_height_mm: i64,
    /// Thinnest structurally credible curtain wall, in millimetres.
    pub min_wall_thickness_mm: i64,
    /// Preferred curtain-wall thickness, in millimetres.
    pub default_wall_thickness_mm: i64,
    /// Maximum height-to-thickness ratio before the wall reads as unstable.
    pub max_slenderness_ratio: f64,
    /// Default total batter inset over the wall height, in millimetres.
    pub default_batter_mm: i64,
    /// Default plinth height, in millimetres.
    pub default_plinth_height_mm: i64,
    /// Minimum comfortable spacing between flanking towers, in metres.
    pub flanking_spacing_min_m: f64,
    /// Maximum flanking distance that still gives bowshot coverage, in metres.
    pub flanking_spacing_max_m: f64,
    /// Plain-language citations shown with validation findings.
    pub source_notes: Vec<String>,
}

impl StylePack {
    /// Load the curated Edwardian Welsh pack from the repository data file.
    pub fn edwardian_welsh() -> Self {
        Self::from_json(EDWARDIAN_WELSH_JSON)
            .expect("styles/edwardian_welsh.json must parse cleanly")
    }

    /// Parse a style pack from JSON. Returns the underlying `serde_json` error
    /// so mod packs can report exactly which field is malformed.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Every style pack compiled into this build, in stable id order.
    pub fn list() -> Vec<StylePack> {
        let mut packs = vec![Self::edwardian_welsh()];
        packs.sort_by(|a, b| a.id.cmp(&b.id));
        packs
    }

    /// Look up a pack by id.
    pub fn find(id: &str) -> Option<Self> {
        Self::list().into_iter().find(|pack| pack.id == id)
    }

    /// First source note, used as the citation on validation findings.
    pub fn primary_source_note(&self) -> &str {
        self.source_notes
            .first()
            .map(String::as_str)
            .unwrap_or("No source note recorded for this style pack.")
    }

    /// Recommended thickness for a wall of `height_mm`, respecting both the
    /// style's minimum and its slenderness limit.
    pub fn recommended_thickness_mm(&self, height_mm: i64) -> i64 {
        let ratio = if self.max_slenderness_ratio > 0.0 {
            self.max_slenderness_ratio
        } else {
            1.0
        };
        let by_slenderness = libm::ceil(height_mm as f64 / ratio) as i64;
        by_slenderness
            .max(self.min_wall_thickness_mm)
            .max(self.default_wall_thickness_mm)
            .max(300)
    }
}

impl Default for StylePack {
    fn default() -> Self {
        Self::edwardian_welsh()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edwardian_pack_loads_and_has_the_expected_rules() {
        let pack = StylePack::edwardian_welsh();
        assert_eq!(pack.id, "edwardian_welsh");
        assert_eq!(pack.name, "Edwardian Welsh (1277–1300)");
        assert_eq!(pack.course_height_mm, 450);
        assert_eq!(pack.min_wall_thickness_mm, 1_800);
        assert_eq!(pack.default_wall_thickness_mm, 2_400);
        assert!((pack.max_slenderness_ratio - 4.5).abs() < f64::EPSILON);
        assert_eq!(pack.default_batter_mm, 900);
        assert_eq!(pack.default_plinth_height_mm, 900);
        assert!((pack.flanking_spacing_min_m - 25.0).abs() < f64::EPSILON);
        assert!((pack.flanking_spacing_max_m - 42.0).abs() < f64::EPSILON);
        assert!(pack
            .source_notes
            .iter()
            .any(|note| note.contains("Caernarfon")));
        assert!(pack.source_notes.iter().any(|note| note.contains("Conwy")));
        assert!(pack
            .source_notes
            .iter()
            .any(|note| note.contains("Harlech")));
        assert!(pack
            .source_notes
            .iter()
            .any(|note| note.contains("Beaumaris")));
    }

    #[test]
    fn list_and_find_are_stable() {
        let packs = StylePack::list();
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].id, "edwardian_welsh");
        assert_eq!(
            StylePack::find("edwardian_welsh").unwrap().name,
            packs[0].name
        );
        assert!(StylePack::find("not_a_style").is_none());
    }

    #[test]
    fn recommended_thickness_obeys_both_limits() {
        let pack = StylePack::edwardian_welsh();
        assert_eq!(pack.recommended_thickness_mm(9_000), 2_400);
        assert_eq!(pack.recommended_thickness_mm(12_000), 2_667);
        assert_eq!(pack.recommended_thickness_mm(1_000), 2_400);
    }
}
