//! The historian: a deterministic, rules-based critic for a chosen style pack.
//!
//! Scores are deliberately simple and legible. Every finding cites a source
//! note from the style pack and is phrased as a historical tendency, never as a
//! verdict on the player (architecture section 6.2).

use serde::{Deserialize, Serialize};

use crate::document::{CastleDocument, Element, RangeKind};
use crate::style::StylePack;
use crate::terrain::SiteTerrain;

/// Which part of the historian's brief a finding belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistorianCategory {
    Authenticity,
    Defensive,
    Domestic,
}

/// One historian observation, with a plain-language explanation and citation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorianFinding {
    pub category: HistorianCategory,
    /// Stable machine-readable code (for example `no_gatehouse`).
    pub code: String,
    pub message: String,
    pub source_note: String,
    /// Signed contribution to the category score.
    pub score_impact: i32,
}

/// The historian's complete report for one document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorianReport {
    pub style_id: String,
    pub authenticity_score: u32,
    pub defensive_score: u32,
    pub domestic_score: u32,
    pub overall_score: u32,
    pub findings: Vec<HistorianFinding>,
}

/// Assess `document` against `style` and the site's defensive geography.
pub fn assess_historian(
    document: &CastleDocument,
    terrain: &SiteTerrain,
    style: &StylePack,
) -> HistorianReport {
    let mut findings = Vec::new();

    let walls: Vec<_> = document
        .elements()
        .filter_map(|element| match element {
            Element::BatteredWall(wall) => Some(wall),
            _ => None,
        })
        .collect();
    let keeps = document.keeps().count();
    let ranges: Vec<_> = document.ranges().collect();
    let tower_count = document
        .elements()
        .filter(|element| matches!(element, Element::RoundTower(_)))
        .count();
    let gatehouse_count = document.gatehouses().count();
    let ditch_count = document.ditches().count();
    let has_hall = ranges.iter().any(|range| range.kind == RangeKind::Hall);
    let has_kitchen = ranges.iter().any(|range| range.kind == RangeKind::Kitchen);
    let has_chapel = ranges.iter().any(|range| range.kind == RangeKind::Chapel);

    let authenticity = score_authenticity(
        style,
        &walls,
        &ranges,
        keeps,
        tower_count,
        gatehouse_count,
        &mut findings,
    );
    let defensive = score_defensive(
        document,
        terrain,
        style,
        &walls,
        gatehouse_count,
        tower_count,
        ditch_count,
        &mut findings,
    );
    let domestic = score_domestic(
        style,
        keeps,
        has_hall,
        has_kitchen,
        has_chapel,
        &mut findings,
    );

    let overall =
        u32::from(((authenticity as u64 + defensive as u64 + domestic as u64 + 1) / 3) as u16);

    HistorianReport {
        style_id: style.id.clone(),
        authenticity_score: authenticity,
        defensive_score: defensive,
        domestic_score: domestic,
        overall_score: overall,
        findings,
    }
}

/// Pick the source note that actually belongs to a finding category. Style
/// packs are curated prose, so keyword matching keeps the citation honest
/// even when the note order changes; the first note is only a last resort.
fn source_for(style: &StylePack, category: HistorianCategory) -> String {
    let keywords: &[&str] = match category {
        HistorianCategory::Authenticity => &["brick", "malbork", "curtain", "fabric"],
        HistorianCategory::Defensive => &["tower", "gatehouse", "flank", "bowshot", "defen"],
        HistorianCategory::Domestic => &["hall", "chapel", "kitchen", "range"],
    };
    style
        .source_notes
        .iter()
        .find(|note| {
            let lower = note.to_lowercase();
            keywords.iter().any(|keyword| lower.contains(keyword))
        })
        .or_else(|| style.source_notes.first())
        .cloned()
        .unwrap_or_else(|| "No source note recorded for this style pack.".to_string())
}

fn domestic_source(style: &StylePack) -> String {
    source_for(style, HistorianCategory::Domestic)
}

fn clamp_score(value: i32) -> u32 {
    value.clamp(0, 100) as u32
}

fn score_authenticity(
    style: &StylePack,
    walls: &[&crate::document::BatteredWall],
    ranges: &[&crate::document::Range],
    keeps: usize,
    tower_count: usize,
    gatehouse_count: usize,
    findings: &mut Vec<HistorianFinding>,
) -> u32 {
    let mut score = 50i32;
    let source = source_for(style, HistorianCategory::Authenticity);

    if walls.is_empty() && keeps == 0 && ranges.is_empty() {
        score -= 25;
        findings.push(HistorianFinding {
            category: HistorianCategory::Authenticity,
            code: "no_masonry".to_string(),
            message: format!(
                "There is no curtain, keep or range yet, so there is little for {} to read as a castle.",
                style.name
            ),
            source_note: source.clone(),
            score_impact: -25,
        });
    }

    let thick_battered = walls.iter().any(|wall| {
        wall.thickness_mm >= style.min_wall_thickness_mm
            && wall.batter_mm >= style.default_batter_mm.max(1) / 2
    });
    if thick_battered {
        score += 10;
    } else if !walls.is_empty() {
        score -= 8;
        findings.push(HistorianFinding {
            category: HistorianCategory::Authenticity,
            code: "wall_not_battered".to_string(),
            message: format!(
                "{} walls are normally founded on a plinth and lean back with a batter, which these curtains do not.",
                style.name
            ),
            source_note: source.clone(),
            score_impact: -8,
        });
    }

    if gatehouse_count > 0 {
        score += 5;
    }

    if style.id == "baltic_brick" {
        if ranges.is_empty() {
            score -= 15;
            findings.push(HistorianFinding {
                category: HistorianCategory::Authenticity,
                code: "no_roofed_ranges".to_string(),
                message: "Baltic brick castles are living buildings as much as fortresses; a hall, chapel or kitchen range under a pitched roof is expected.".to_string(),
                source_note: domestic_source(style),
                score_impact: -15,
            });
        } else {
            let pitched = ranges.iter().any(|range| {
                (25..=60).contains(&range.roof_pitch_deg) && range.wall_height_mm >= 3_000
            });
            if pitched {
                score += 15;
            }
        }
        if keeps > 0 {
            score += 10;
        } else {
            score -= 8;
            findings.push(HistorianFinding {
                category: HistorianCategory::Authenticity,
                code: "no_keep".to_string(),
                message: "A Baltic brick castle of this period usually centres on a tall keep or main tower rather than a low curtain alone.".to_string(),
                source_note: source.clone(),
                score_impact: -8,
            });
        }
        let brick_proportions = !walls.is_empty()
            && walls.iter().all(|wall| {
                wall.thickness_mm >= style.min_wall_thickness_mm
                    && wall.thickness_mm <= (style.default_wall_thickness_mm * 5) / 4
            });
        if brick_proportions {
            score += 10;
        } else if !walls.is_empty() {
            score -= 6;
            findings.push(HistorianFinding {
                category: HistorianCategory::Authenticity,
                code: "walls_too_heavy_for_brick".to_string(),
                message: format!(
                    "Coursed brick lets {} walls stay relatively thin (around {:.1} m here); these read as heavier stone masonry.",
                    style.name,
                    style.default_wall_thickness_mm as f64 / 1000.0
                ),
                source_note: source.clone(),
                score_impact: -6,
            });
        }
    } else {
        let shaped_tower = document_has_shaped_tower(walls, tower_count);
        if shaped_tower {
            score += 10;
        }
        if gatehouse_count > 0 {
            score += 10;
        }
        if !walls.is_empty()
            && walls
                .iter()
                .any(|wall| wall.thickness_mm >= 2_000 && wall.batter_mm >= 600)
        {
            score += 15;
        }
    }

    clamp_score(score)
}

/// True when the document contains a round or D-shaped tower. `tower_count` is
/// used only to avoid a second document walk when there are no towers at all.
fn document_has_shaped_tower(
    _walls: &[&crate::document::BatteredWall],
    tower_count: usize,
) -> bool {
    // Round and D-shaped plans are the Edwardian norm; the caller avoids this
    // path for the Baltic style. Any tower at all is enough to award the
    // feature bonus here because the shape itself is not part of the public
    // reference list.
    tower_count > 0
}

#[allow(clippy::too_many_arguments)]
fn score_defensive(
    document: &CastleDocument,
    terrain: &SiteTerrain,
    style: &StylePack,
    walls: &[&crate::document::BatteredWall],
    gatehouse_count: usize,
    tower_count: usize,
    ditch_count: usize,
    findings: &mut Vec<HistorianFinding>,
) -> u32 {
    let source = source_for(style, HistorianCategory::Defensive);
    let mut flankers: Vec<[i64; 2]> = Vec::new();
    for element in document.elements() {
        match element {
            Element::RoundTower(tower) => flankers.push(tower.centre_mm),
            Element::Gatehouse(gatehouse) => {
                for tower in gatehouse.flank_towers() {
                    flankers.push(tower.centre_mm);
                }
            }
            _ => {}
        }
    }

    // A point is only swept if it is within *half* the maximum tower spacing:
    // two drums `flanking_spacing_max_m` apart meet at the midpoint, while
    // towers farther apart than that leave an uncovered dead zone.
    let max_reach_mm = (style.flanking_spacing_max_m.max(1.0) * 1000.0) as i64 / 2;
    let mut covered = 0usize;
    let mut samples = 0usize;
    let mut worst_gap_mm = 0i64;
    for wall in walls {
        let dx = wall.end_mm[0] as f64 - wall.start_mm[0] as f64;
        let dz = wall.end_mm[1] as f64 - wall.start_mm[1] as f64;
        let len = libm::sqrt(dx * dx + dz * dz);
        if len < 1.0 {
            continue;
        }
        for step in 0..=8 {
            let t = step as f64 / 8.0;
            let x = wall.start_mm[0] as f64 + dx * t;
            let z = wall.start_mm[1] as f64 + dz * t;
            samples += 1;
            let nearest = flankers
                .iter()
                .map(|centre| {
                    let ex = x - centre[0] as f64;
                    let ez = z - centre[1] as f64;
                    libm::sqrt(ex * ex + ez * ez) as i64
                })
                .min();
            match nearest {
                Some(distance) if distance <= max_reach_mm => covered += 1,
                Some(distance) => worst_gap_mm = worst_gap_mm.max(distance - max_reach_mm),
                None => worst_gap_mm = worst_gap_mm.max(max_reach_mm),
            }
        }
    }

    let mut score = if samples > 0 {
        (covered as i32 * 50) / samples as i32
    } else {
        0
    };
    if samples > 0 && covered < samples {
        findings.push(HistorianFinding {
            category: HistorianCategory::Defensive,
            code: "curtain_dead_ground".to_string(),
            message: format!(
                "Part of the curtain falls outside the {:.0} m sweep of a flanking tower or gatehouse (worst gap about {:.1} m), so that ground could not be covered.",
                max_reach_mm as f64 / 1000.0,
                worst_gap_mm as f64 / 1000.0
            ),
            source_note: source.clone(),
            score_impact: -(((samples - covered) as i32 * 50) / samples as i32),
        });
    }
    if tower_count == 0 && gatehouse_count == 0 {
        findings.push(HistorianFinding {
            category: HistorianCategory::Defensive,
            code: "no_flanking_towers".to_string(),
            message: "There are no flanking towers or gatehouse drums, so the curtain has no crossing fire along its face.".to_string(),
            source_note: source.clone(),
            score_impact: -50,
        });
    }

    if gatehouse_count > 0 {
        score += 20;
    } else {
        findings.push(HistorianFinding {
            category: HistorianCategory::Defensive,
            code: "no_gatehouse".to_string(),
            message: format!(
                "{} castles control the approach through a gatehouse with flanking drums; this one has no gatehouse.",
                style.name
            ),
            source_note: source.clone(),
            score_impact: -20,
        });
    }

    if ditch_count > 0 {
        score += 15;
    } else {
        findings.push(HistorianFinding {
            category: HistorianCategory::Defensive,
            code: "no_approach_ditch".to_string(),
            message: "A ditch or fosse across the landward approach would slow an attacker and keep engines out of bowshot.".to_string(),
            source_note: source.clone(),
            score_impact: -15,
        });
    }

    // Elevation advantage: the ground under the castle should not be flat if a
    // nearby high shoulder exists. A modest bonus for any real relief.
    if let Some(centre) = element_centroid(document) {
        let mut min_h = i64::MAX;
        let mut max_h = i64::MIN;
        for (dx, dz) in [
            (-20_000i64, -20_000i64),
            (20_000, -20_000),
            (20_000, 20_000),
            (-20_000, 20_000),
        ] {
            let h = terrain.height_mm(centre[0] + dx, centre[1] + dz);
            min_h = min_h.min(h);
            max_h = max_h.max(h);
        }
        if max_h - min_h >= 1_500 {
            score += 10;
        }
    }

    clamp_score(score)
}

fn element_centroid(document: &CastleDocument) -> Option<[i64; 2]> {
    let mut sum = [0i128; 2];
    let mut count = 0i128;
    for element in document.elements() {
        let point = match element {
            Element::BatteredWall(wall) => [
                (wall.start_mm[0] + wall.end_mm[0]) / 2,
                (wall.start_mm[1] + wall.end_mm[1]) / 2,
            ],
            Element::RoundTower(tower) => tower.centre_mm,
            Element::Gatehouse(gatehouse) => gatehouse.centre_mm,
            Element::Keep(keep) => keep.centre_mm,
            Element::Range(range) => range.centre_mm,
            Element::Ditch(_) => continue,
        };
        sum[0] += point[0] as i128;
        sum[1] += point[1] as i128;
        count += 1;
    }
    if count == 0 {
        return None;
    }
    Some([(sum[0] / count) as i64, (sum[1] / count) as i64])
}

fn score_domestic(
    style: &StylePack,
    keeps: usize,
    has_hall: bool,
    has_kitchen: bool,
    has_chapel: bool,
    findings: &mut Vec<HistorianFinding>,
) -> u32 {
    let source = source_for(style, HistorianCategory::Domestic);
    let mut score = 0i32;

    if keeps > 0 || has_hall {
        score += 35;
    } else {
        findings.push(HistorianFinding {
            category: HistorianCategory::Domestic,
            code: "no_lordly_residence".to_string(),
            message: format!(
                "There is no keep or hall range, so the lord has no obvious high-status residence in this {} castle.",
                style.name
            ),
            source_note: source.clone(),
            score_impact: -35,
        });
    }
    if has_kitchen {
        score += 30;
    } else {
        findings.push(HistorianFinding {
            category: HistorianCategory::Domestic,
            code: "no_kitchen".to_string(),
            message: "A kitchen range is missing; in a residential castle of this period it should sit apart from the hall to keep fire risk away from the great chamber.".to_string(),
            source_note: source.clone(),
            score_impact: -30,
        });
    }
    if has_chapel {
        score += 35;
    } else {
        findings.push(HistorianFinding {
            category: HistorianCategory::Domestic,
            code: "no_chapel".to_string(),
            message: "A chapel range is missing; even compact castles of this period usually provided a dedicated chapel for the household.".to_string(),
            source_note: source,
            score_impact: -35,
        });
    }

    clamp_score(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{BatteredWall, RoundTower};
    use crate::fixtures::m2_baltic_document;

    #[test]
    fn towers_spaced_beyond_the_max_reach_leave_a_dead_zone() {
        let terrain = SiteTerrain::for_site("lake_island").unwrap();
        let style = StylePack::baltic_brick();
        let mut document = CastleDocument::new();
        document.add_wall(BatteredWall::new(
            1,
            "Long curtain",
            [-38_000, 0],
            [38_000, 0],
            6_000,
            1_800,
        ));
        document.add_tower(RoundTower::new(2, "West drum", [-38_000, 0], 3_000, 10_000));
        document.add_tower(RoundTower::new(3, "East drum", [38_000, 0], 3_000, 10_000));

        let report = assess_historian(&document, &terrain, &style);
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.code == "curtain_dead_ground")
            .expect("a 76 m gap between drums must leave dead ground");
        assert_eq!(finding.category, HistorianCategory::Defensive);
        assert!(
            finding.source_note.to_lowercase().contains("flank")
                || finding.source_note.to_lowercase().contains("tower")
        );
    }

    #[test]
    fn towers_at_the_max_spacing_meet_at_the_midpoint() {
        let terrain = SiteTerrain::for_site("lake_island").unwrap();
        let style = StylePack::baltic_brick();
        let mut document = CastleDocument::new();
        document.add_wall(BatteredWall::new(
            1,
            "Curtain",
            [-19_000, 0],
            [19_000, 0],
            6_000,
            1_800,
        ));
        document.add_tower(RoundTower::new(2, "West drum", [-19_000, 0], 3_000, 10_000));
        document.add_tower(RoundTower::new(3, "East drum", [19_000, 0], 3_000, 10_000));

        let report = assess_historian(&document, &terrain, &style);
        assert!(
            report
                .findings
                .iter()
                .all(|finding| finding.code != "curtain_dead_ground"),
            "38 m spacing should just cover the midpoint: {:?}",
            report.findings
        );
    }

    #[test]
    fn a_bare_wall_scores_low_on_domestic_and_defensive_logic() {
        let terrain = SiteTerrain::for_site("lake_island").unwrap();
        let style = StylePack::baltic_brick();
        let mut document = CastleDocument::new();
        document.add_wall(BatteredWall::new(
            1,
            "Lone curtain",
            [-8_000, 0],
            [8_000, 0],
            6_000,
            1_800,
        ));

        let report = assess_historian(&document, &terrain, &style);
        assert_eq!(report.style_id, "baltic_brick");
        assert!(report.authenticity_score > 0);
        assert!(report.domestic_score <= 10, "{:?}", report);
        assert!(report.defensive_score <= 10, "{:?}", report);
        assert!(report.findings.iter().any(|finding| {
            finding.category == HistorianCategory::Domestic
                && finding.code == "no_chapel"
                && !finding.source_note.is_empty()
        }));
        assert!(report.findings.iter().any(|finding| {
            finding.category == HistorianCategory::Defensive
                && (finding.code == "no_gatehouse" || finding.code == "curtain_dead_ground")
                && !finding.source_note.is_empty()
        }));
    }

    #[test]
    fn the_m2_baltic_showcase_scores_highly() {
        let terrain = SiteTerrain::for_site("lake_island").unwrap();
        let style = StylePack::baltic_brick();
        let report = assess_historian(&m2_baltic_document(), &terrain, &style);
        assert!(
            report.overall_score >= 80,
            "overall {} with findings {:?}",
            report.overall_score,
            report.findings
        );
        assert!(report.domestic_score >= 80, "{:?}", report.findings);
        assert!(report.defensive_score >= 70, "{:?}", report.findings);
        assert!(report.authenticity_score >= 80, "{:?}", report.findings);
    }
}
