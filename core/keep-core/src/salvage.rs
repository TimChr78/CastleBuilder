//! Mid-build edit salvage diffing.
//!
//! A player may change their mind while a castle is still rising: a wall gets
//! taller, a tower moves, a range is pulled down. Stone that was already laid
//! cannot simply vanish, so the commission system credits part of its value
//! back when it is dismantled and keeps the rest. This module compares the
//! deterministic masonry layout of the old and new documents and reports what
//! was kept, what came down and what was newly laid.
//!
//! Everything here works on quantised integer millimetre block identities so a
//! document written and reloaded on another platform produces exactly the same
//! diff, never a float-drift near-miss.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::document::CastleDocument;
use crate::masonry::generate_blocks;
use crate::terrain::SiteTerrain;

/// Silver recovered per hundred dismantled blocks, at full value.
pub const SILVER_PER_100_BLOCKS: i64 = 10;

/// Percentage of a dismantled block's original value recovered as salvage.
pub const SALVAGE_RECOVERY_PERCENT: i64 = 60;

/// Summary of what a mid-build edit did to already-laid stone.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SalvageDiff {
    /// Already-built old blocks that still exist identically.
    pub unchanged_built_blocks: usize,
    /// Already-built old blocks that are no longer present.
    pub dismantled_blocks: usize,
    /// New-document blocks that were not part of the old built set.
    pub added_blocks: usize,
    /// Silver credited back for dismantled stone.
    pub salvage_credit_silver: i64,
    /// Fraction of the old built stone that survived, `0.0..=1.0`.
    pub preserved_fraction: f32,
}

/// Deterministic integer identity of one masonry block.
///
/// `(element_id, course, centre x/y/z, width, height)`, all in whole
/// millimetres. Build order and floating-point centre rounding are excluded so
/// a purely visual re-ordering cannot change the diff.
type BlockKey = (u64, i32, i64, i64, i64, i64, i64);

fn block_key(block: &crate::masonry::Block) -> BlockKey {
    let centre = block.centre_mm();
    (
        block.element_id,
        block.course_index,
        libm::round(centre[0]) as i64,
        libm::round(centre[1]) as i64,
        libm::round(centre[2]) as i64,
        block.block_width_mm,
        block.block_height_mm,
    )
}

/// Every block the document's explicit earthworks would produce.
fn block_keys(document: &CastleDocument, terrain: &SiteTerrain) -> BTreeSet<BlockKey> {
    let ditches: Vec<&crate::document::Ditch> = document.ditches().collect();
    let sculpts: Vec<&crate::document::TerrainSculpt> = document.terrain_sculpts().collect();
    let effective = terrain.with_earthworks(&ditches, &sculpts);
    generate_blocks(document, &effective)
        .iter()
        .map(block_key)
        .collect()
}

/// Blocks in `document` whose normalised build index is at or below
/// `build_front_norm`. A front of `0.0` keeps only the first stone; `1.0`
/// keeps every stone.
fn built_block_keys(
    document: &CastleDocument,
    terrain: &SiteTerrain,
    build_front_norm: f32,
) -> BTreeSet<BlockKey> {
    let ditches: Vec<&crate::document::Ditch> = document.ditches().collect();
    let sculpts: Vec<&crate::document::TerrainSculpt> = document.terrain_sculpts().collect();
    let effective = terrain.with_earthworks(&ditches, &sculpts);
    let blocks = generate_blocks(document, &effective);
    let total = blocks.len();
    if total == 0 {
        return BTreeSet::new();
    }
    let front = build_front_norm.clamp(0.0, 1.0) as f64;
    blocks
        .iter()
        .filter(|block| {
            let normalised = block.build_index as f64 / total as f64;
            normalised <= front
        })
        .map(block_key)
        .collect()
}

/// Compare the already-built stone of `old_doc` with the full layout of
/// `new_doc`, returning the salvage accounting for a mid-build edit.
pub fn diff_mid_build_edit(
    old_doc: &CastleDocument,
    new_doc: &CastleDocument,
    terrain: &SiteTerrain,
    build_front_norm: f32,
) -> SalvageDiff {
    let old_built = built_block_keys(old_doc, terrain, build_front_norm);
    let new_blocks = block_keys(new_doc, terrain);

    let unchanged_built_blocks = old_built.intersection(&new_blocks).count();
    let dismantled_blocks = old_built.difference(&new_blocks).count();
    let added_blocks = new_blocks.difference(&old_built).count();

    let total_old_built = old_built.len();
    let preserved_fraction = if total_old_built == 0 {
        1.0
    } else {
        unchanged_built_blocks as f32 / total_old_built as f32
    };

    let mut salvage_credit_silver =
        (dismantled_blocks as i64 * SILVER_PER_100_BLOCKS * SALVAGE_RECOVERY_PERCENT) / (100 * 100);
    if dismantled_blocks > 0 && salvage_credit_silver < 1 {
        salvage_credit_silver = 1;
    }

    SalvageDiff {
        unchanged_built_blocks,
        dismantled_blocks,
        added_blocks,
        salvage_credit_silver,
        preserved_fraction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::BatteredWall;

    fn wall_document(height_mm: i64) -> CastleDocument {
        let mut document = CastleDocument::new();
        document.add_wall(BatteredWall::new(
            1,
            "Curtain",
            [-15_000, 0],
            [15_000, 0],
            height_mm,
            2_400,
        ));
        document
    }

    #[test]
    fn raising_a_wall_mid_build_preserves_the_lower_courses() {
        let terrain = SiteTerrain::m0a_promontory();
        let old = wall_document(12_000);
        let new = wall_document(20_000);

        let diff = diff_mid_build_edit(&old, &new, &terrain, 0.5);
        assert!(
            diff.unchanged_built_blocks > 0,
            "expected the lower half to survive: {diff:?}"
        );
        assert!(diff.preserved_fraction > 0.0, "{diff:?}");
        assert!(
            diff.preserved_fraction <= 1.0,
            "a fraction must not exceed one: {diff:?}"
        );
        assert!(diff.added_blocks > 0, "the new upper courses are added");
    }

    #[test]
    fn deleting_an_element_mid_build_yields_salvage_credit() {
        let terrain = SiteTerrain::m0a_promontory();
        let old = wall_document(9_000);
        let new = CastleDocument::new();

        let diff = diff_mid_build_edit(&old, &new, &terrain, 1.0);
        assert!(diff.dismantled_blocks > 0, "{diff:?}");
        assert!(diff.salvage_credit_silver > 0, "{diff:?}");
        assert_eq!(diff.unchanged_built_blocks, 0);
        assert_eq!(diff.preserved_fraction, 0.0);
    }

    #[test]
    fn identical_documents_preserve_everything() {
        let terrain = SiteTerrain::m0a_promontory();
        let old = wall_document(11_000);
        let new = old.clone();

        let diff = diff_mid_build_edit(&old, &new, &terrain, 1.0);
        assert_eq!(diff.dismantled_blocks, 0);
        assert_eq!(diff.added_blocks, 0);
        assert_eq!(diff.salvage_credit_silver, 0);
        assert_eq!(diff.preserved_fraction, 1.0);
        assert!(diff.unchanged_built_blocks > 0);
    }

    #[test]
    fn diffing_is_deterministic_and_quantised() {
        let terrain = SiteTerrain::m0a_promontory();
        let old = wall_document(10_000);
        let new = wall_document(16_000);

        let first = diff_mid_build_edit(&old, &new, &terrain, 0.6);
        let second = diff_mid_build_edit(&old, &new, &terrain, 0.6);
        assert_eq!(first, second);
    }
}
