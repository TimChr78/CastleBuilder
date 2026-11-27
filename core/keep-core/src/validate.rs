//! Structural plausibility checks.
//!
//! These are the checks a medieval master mason would recognise, not finite
//! elements (architecture section 6.1): wall slenderness, footing on soil or a
//! steep slope, and openings that are too low or too tall. Every finding is
//! plain language, cites the style pack's source notes, and where possible
//! carries a one-click `suggested_fix` command that can be applied through the
//! normal session batch API.

use serde::{Deserialize, Serialize};

use crate::command::{CommandOp, MAX_WALL_THICKNESS_MM, MIN_WALL_HEIGHT_MM};
use crate::document::{CastleDocument, Element, ElementId};
use crate::style::StylePack;
use crate::terrain::SiteTerrain;

/// How serious a structural finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// One structural finding with a plain-language explanation and optional fix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub element_id: ElementId,
    pub severity: Severity,
    /// Stable machine-readable code (for example `wall_too_slender`).
    pub code: String,
    pub message: String,
    pub source_note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_fix: Option<CommandOp>,
}

/// Result of a structural assessment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssessmentReport {
    pub findings: Vec<Finding>,
    /// 0..=100; errors cost more than warnings.
    pub score: u32,
    /// True when there are no [`Severity::Error`] findings.
    pub passed: bool,
}

impl AssessmentReport {
    pub fn error_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == Severity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == Severity::Warning)
            .count()
    }
}

/// Assess the document against the style's structural rules and the site
/// terrain.
pub fn validate_structure(
    document: &CastleDocument,
    terrain: &SiteTerrain,
    style: &StylePack,
) -> AssessmentReport {
    let mut findings = Vec::new();
    let source = style.primary_source_note().to_string();

    // Footing, bedrock and slope checks must see the ground the player actually
    // carved, not the pristine site. `with_ditches` returns a copy when there
    // are no earthworks, so this stays cheap.
    let ditches: Vec<&crate::document::Ditch> = document.ditches().collect();
    let effective_terrain = terrain.with_ditches(&ditches);
    let terrain = &effective_terrain;

    for element in document.elements() {
        match element {
            Element::BatteredWall(wall) => {
                let height = wall.height_mm.max(1);
                let thickness = wall.thickness_mm.max(1);
                let slender = height as f64 / thickness as f64;
                if thickness < style.min_wall_thickness_mm || slender > style.max_slenderness_ratio
                {
                    let ratio = if style.max_slenderness_ratio > 0.0 {
                        style.max_slenderness_ratio
                    } else {
                        1.0
                    };
                    let recommended = style
                        .recommended_thickness_mm(height)
                        .min(MAX_WALL_THICKNESS_MM);
                    // A very tall wall can still be too slender at the maximum
                    // legal thickness. In that case the one-click fix must also
                    // clamp the height, otherwise applying it would leave the
                    // finding in place (or be rejected by the command bounds).
                    let max_fixable_height = (MAX_WALL_THICKNESS_MM as f64 * ratio).floor() as i64;
                    let still_slender = height as f64 / recommended as f64 > ratio;
                    let fix_height = still_slender.then(|| height.min(max_fixable_height));
                    let fix_is_possible = fix_height.is_none_or(|h| h >= MIN_WALL_HEIGHT_MM);
                    let fix_description = match fix_height {
                        Some(h) if fix_is_possible => format!(
                            "thicken it to {:.2} m and reduce its height to {:.1} m",
                            recommended as f64 / 1000.0,
                            h as f64 / 1000.0
                        ),
                        _ => format!("thicken it to {:.2} m", recommended as f64 / 1000.0),
                    };
                    findings.push(Finding {
                        element_id: wall.id,
                        severity: Severity::Error,
                        code: "wall_too_slender".to_string(),
                        message: format!(
                            "This {:.1} m wall is only {:.1} m thick. {} expects at least {} m, and a height-to-thickness ratio no greater than {:.1}; {}.",
                            height as f64 / 1000.0,
                            thickness as f64 / 1000.0,
                            style.name,
                            style.min_wall_thickness_mm as f64 / 1000.0,
                            style.max_slenderness_ratio,
                            fix_description
                        ),
                        source_note: source.clone(),
                        suggested_fix: fix_is_possible.then_some(CommandOp::ModifyElement {
                            id: wall.id,
                            height_mm: fix_height,
                            thickness_mm: Some(recommended),
                            batter_mm: None,
                            plinth_height_mm: None,
                            plan: None,
                            orientation_deg: None,
                        }),
                    });
                }

                // Footing: soil under the footprint needs a plinth; a steep
                // drop along the run needs a batter.
                let start = terrain.height_mm(wall.start_mm[0], wall.start_mm[1]);
                let end = terrain.height_mm(wall.end_mm[0], wall.end_mm[1]);
                let mid = terrain.height_mm(
                    (wall.start_mm[0] + wall.end_mm[0]) / 2,
                    (wall.start_mm[1] + wall.end_mm[1]) / 2,
                );
                let soil = !terrain.bedrock_mask(wall.start_mm[0], wall.start_mm[1])
                    || !terrain.bedrock_mask(wall.end_mm[0], wall.end_mm[1])
                    || !terrain.bedrock_mask(
                        (wall.start_mm[0] + wall.end_mm[0]) / 2,
                        (wall.start_mm[1] + wall.end_mm[1]) / 2,
                    );
                let drop = start.max(mid).max(end) - start.min(mid).min(end);
                let needs_plinth = soil && wall.plinth_height_mm == 0;
                let needs_batter = drop > 2_500 && wall.batter_mm == 0;
                if needs_plinth || needs_batter {
                    let mut message = String::from("This wall stands on ");
                    if needs_plinth && needs_batter {
                        message.push_str("soil and a steep slope without a plinth or batter");
                    } else if needs_plinth {
                        message.push_str("soil without a projecting plinth");
                    } else {
                        message.push_str("a steep slope without a battered face");
                    }
                    message.push_str(&format!(
                        ". {} walls are founded on a plinth and lean back with a batter.",
                        style.name
                    ));
                    findings.push(Finding {
                        element_id: wall.id,
                        severity: Severity::Warning,
                        code: "wall_footing".to_string(),
                        message,
                        source_note: source.clone(),
                        suggested_fix: Some(CommandOp::ModifyElement {
                            id: wall.id,
                            height_mm: None,
                            thickness_mm: None,
                            batter_mm: needs_batter.then_some(style.default_batter_mm),
                            plinth_height_mm: needs_plinth
                                .then_some(style.default_plinth_height_mm),
                            plan: None,
                            orientation_deg: None,
                        }),
                    });
                }
            }
            Element::Gatehouse(gatehouse) => {
                if gatehouse.passage_height_mm >= gatehouse.height_mm {
                    findings.push(Finding {
                        element_id: gatehouse.id,
                        severity: Severity::Error,
                        code: "gatehouse_passage_too_tall".to_string(),
                        message: format!(
                            "The passage is {:.1} m high but the gatehouse is only {:.1} m high, so there is no masonry above the vault.",
                            gatehouse.passage_height_mm as f64 / 1000.0,
                            gatehouse.height_mm as f64 / 1000.0
                        ),
                        source_note: source.clone(),
                        suggested_fix: None,
                    });
                }
            }
            Element::Keep(keep) => {
                let height = keep.height_mm.max(1);
                let thickness = keep.wall_thickness_mm.max(1);
                let slender = height as f64 / thickness as f64;
                // Keeps carry the parapet and the lord's residence, so their
                // walls are checked against the same style minimum as a
                // curtain but with a tighter 8.5:1 slenderness limit.
                if thickness < style.min_wall_thickness_mm || slender > 8.5 {
                    let recommended = ((height as f64 / 8.5).ceil() as i64)
                        .max(style.min_wall_thickness_mm)
                        .max(style.default_wall_thickness_mm);
                    let suggested_fix = (recommended <= MAX_WALL_THICKNESS_MM).then_some(
                        CommandOp::ModifyElement {
                            id: keep.id,
                            height_mm: None,
                            thickness_mm: Some(recommended),
                            batter_mm: None,
                            plinth_height_mm: None,
                            plan: None,
                            orientation_deg: None,
                        },
                    );
                    findings.push(Finding {
                        element_id: keep.id,
                        severity: Severity::Error,
                        code: "keep_too_slender".to_string(),
                        message: format!(
                            "This {:.1} m keep has {:.1} m thick walls. {} expects a keep wall of at least {} m and a height-to-thickness ratio no greater than 8.5; a keep this tall needs walls around {:.1} m thick.",
                            height as f64 / 1000.0,
                            thickness as f64 / 1000.0,
                            style.name,
                            style.min_wall_thickness_mm as f64 / 1000.0,
                            recommended as f64 / 1000.0
                        ),
                        source_note: source.clone(),
                        suggested_fix,
                    });
                }
            }
            Element::Range(range) => {
                // A medieval timber roof truss is credible to roughly 16 m of
                // unsupported span (architecture section 6.1). Wider ranges
                // need an aisle, a stone vault or a crown-post alternative.
                if range.width_mm > 16_000 {
                    findings.push(Finding {
                        element_id: range.id,
                        severity: Severity::Warning,
                        code: "range_roof_span_too_wide".to_string(),
                        message: format!(
                            "This range is {:.1} m across, beyond the roughly 16 m an unsupported timber roof truss could span in {}; it would need an aisle, a stone vault or an internal arcade.",
                            range.width_mm as f64 / 1000.0,
                            style.name
                        ),
                        source_note: source.clone(),
                        suggested_fix: None,
                    });
                }
            }
            Element::RoundTower(_) | Element::Ditch(_) => {}
        }
    }

    for opening in document.openings() {
        let Some(Element::BatteredWall(wall)) = document.get(opening.wall_id) else {
            continue;
        };
        if opening.sill_height_mm < 900 {
            findings.push(Finding {
                element_id: opening.wall_id,
                severity: Severity::Warning,
                code: "opening_sill_too_low".to_string(),
                message: format!(
                    "An opening with a sill at {:.1} m is low enough to be climbed or undermined; loops in this style normally sit at least 0.9 m above the ground.",
                    opening.sill_height_mm as f64 / 1000.0
                ),
                source_note: source.clone(),
                suggested_fix: None,
            });
        }
        if opening.height_mm > wall.height_mm.max(1) / 3 {
            findings.push(Finding {
                element_id: opening.wall_id,
                severity: Severity::Warning,
                code: "opening_too_tall".to_string(),
                message: format!(
                    "A {:.1} m opening is tall for a {:.1} m wall; it removes too much of the wall's strength low down.",
                    opening.height_mm as f64 / 1000.0,
                    wall.height_mm as f64 / 1000.0
                ),
                source_note: source.clone(),
                suggested_fix: None,
            });
        }
    }

    let errors = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Error)
        .count();
    let warnings = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Warning)
        .count();
    let penalty = (errors * 25 + warnings * 8) as u32;
    let score = 100u32.saturating_sub(penalty);
    AssessmentReport {
        findings,
        score,
        passed: errors == 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::AuthorId;
    use crate::document::{BatteredWall, Opening};
    use crate::session::Session;

    fn style() -> StylePack {
        StylePack::edwardian_welsh()
    }

    #[test]
    fn a_wide_range_warns_about_its_timber_roof_span() {
        use crate::document::{Range, RangeKind};

        let terrain = SiteTerrain::m0a_promontory();
        let mut document = CastleDocument::new();
        let mut range = Range::new(20, "Wide hall", RangeKind::Hall, [0, 0], 0);
        range.width_mm = 20_000;
        document.add_range(range);

        let report = validate_structure(&document, &terrain, &style());
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.code == "range_roof_span_too_wide")
            .expect("roof span finding");
        assert_eq!(finding.severity, Severity::Warning);
        assert!(report.passed, "a span warning must not fail the castle");
    }

    #[test]
    fn a_slender_keep_is_an_error_with_a_thicker_wall_fix() {
        use crate::document::Keep;

        let terrain = SiteTerrain::m0a_promontory();
        let mut document = CastleDocument::new();
        let mut keep = Keep::new(21, "Needle keep", [0, 0], 0);
        keep.height_mm = 30_000;
        keep.wall_thickness_mm = 2_000;
        document.add_keep(keep);

        let report = validate_structure(&document, &terrain, &style());
        assert!(!report.passed);
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.code == "keep_too_slender")
            .expect("keep slenderness finding");
        assert_eq!(finding.severity, Severity::Error);
        let Some(CommandOp::ModifyElement { thickness_mm, .. }) = finding.suggested_fix.as_ref()
        else {
            panic!("expected a ModifyElement suggested fix");
        };
        assert!(thickness_mm.unwrap() >= 3_529);
    }

    #[test]
    fn a_sound_wall_passes_with_a_full_score() {
        let terrain = SiteTerrain::m0a_promontory();
        let mut document = CastleDocument::new();
        let mut wall = BatteredWall::new(1, "Sound wall", [-8_000, 0], [8_000, 0], 9_000, 2_400);
        wall.batter_mm = 900;
        wall.plinth_height_mm = 900;
        document.add_wall(wall);

        let report = validate_structure(&document, &terrain, &style());
        assert!(report.passed);
        assert_eq!(report.score, 100);
        assert!(report.findings.is_empty());
    }

    #[test]
    fn a_thin_tall_wall_is_an_error_with_a_usable_fix() {
        let terrain = SiteTerrain::m0a_promontory();
        let mut document = CastleDocument::new();
        let mut wall = BatteredWall::new(1, "Thin wall", [-6_000, 0], [6_000, 0], 12_000, 1_000);
        wall.batter_mm = 900;
        wall.plinth_height_mm = 900;
        document.add_wall(wall);

        let report = validate_structure(&document, &terrain, &style());
        assert!(!report.passed);
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.code == "wall_too_slender")
            .expect("slenderness finding");
        assert_eq!(finding.severity, Severity::Error);
        let Some(CommandOp::ModifyElement {
            id, thickness_mm, ..
        }) = finding.suggested_fix.as_ref()
        else {
            panic!("expected a ModifyElement suggested fix");
        };
        assert_eq!(*id, 1);
        let recommended = thickness_mm.expect("recommended thickness");
        assert!(recommended >= 2_667);
    }

    #[test]
    fn low_and_tall_openings_warn() {
        let terrain = SiteTerrain::m0a_promontory();
        let mut document = CastleDocument::new();
        document.add_wall(BatteredWall::new(
            1,
            "Wall",
            [-6_000, 0],
            [6_000, 0],
            6_000,
            2_400,
        ));
        document.add_opening(Opening::arrow_loop(10, 1, "Low loop", 500, 400));

        let report = validate_structure(&document, &terrain, &style());
        assert!(report.passed, "warnings alone must not fail the wall");
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "opening_sill_too_low"));
    }

    #[test]
    fn a_60m_wall_gets_a_clamped_fix_that_actually_validates() {
        let terrain = SiteTerrain::m0a_promontory();
        let mut document = CastleDocument::new();
        let mut wall = BatteredWall::new(1, "Needle wall", [-6_000, 0], [6_000, 0], 60_000, 1_000);
        wall.batter_mm = 900;
        wall.plinth_height_mm = 900;
        document.add_wall(wall);

        let report = validate_structure(&document, &terrain, &style());
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.code == "wall_too_slender")
            .expect("slenderness finding");
        let Some(CommandOp::ModifyElement {
            height_mm,
            thickness_mm,
            ..
        }) = finding.suggested_fix.as_ref()
        else {
            panic!("expected a ModifyElement suggested fix");
        };
        let thickness = thickness_mm.expect("recommended thickness");
        let height = height_mm.expect("clamped height");
        assert!(thickness <= crate::command::MAX_WALL_THICKNESS_MM);
        assert!(height < 60_000);
        let fix = finding.suggested_fix.clone().unwrap();
        fix.validated()
            .expect("clamped fix must pass command bounds");

        let mut session = Session::from_document("m0a_promontory", terrain.clone(), document);
        session
            .apply_batch(AuthorId::player("mason").unwrap(), vec![fix])
            .unwrap();
        let after = validate_structure(session.document(), &terrain, &style());
        assert!(
            after
                .findings
                .iter()
                .all(|finding| finding.code != "wall_too_slender"),
            "findings after fix: {:?}",
            after.findings
        );
    }

    #[test]
    fn a_ditch_under_an_unplinthed_wall_triggers_a_footing_finding() {
        use crate::document::Ditch;

        let terrain = SiteTerrain::m0a_promontory();
        let mut document = CastleDocument::new();
        let mut wall = BatteredWall::new(1, "Wall", [-10_000, 0], [10_000, 0], 8_000, 2_400);
        wall.batter_mm = 900;
        wall.plinth_height_mm = 0;
        document.add_wall(wall);

        let before = validate_structure(&document, &terrain, &style());
        assert!(
            before
                .findings
                .iter()
                .all(|finding| finding.code != "wall_footing"),
            "bedrock wall should not need a plinth before the ditch: {:?}",
            before.findings
        );

        document.add_ditch(Ditch::new(
            2,
            "Ford ditch",
            [-10_000, 0],
            [10_000, 0],
            4_000,
            2_000,
        ));
        let after = validate_structure(&document, &terrain, &style());
        let footing = after
            .findings
            .iter()
            .find(|finding| finding.code == "wall_footing")
            .expect("ditch-carved soil must trigger the footing check");
        assert_eq!(footing.severity, Severity::Warning);
        assert!(matches!(
            footing.suggested_fix,
            Some(CommandOp::ModifyElement {
                plinth_height_mm: Some(_),
                ..
            })
        ));
    }

    #[test]
    fn applying_the_suggested_fix_through_the_session_makes_the_wall_pass() {
        let terrain = SiteTerrain::m0a_promontory();
        let mut document = CastleDocument::new();
        let mut wall = BatteredWall::new(1, "Thin wall", [-6_000, 0], [6_000, 0], 12_000, 1_000);
        wall.batter_mm = 900;
        wall.plinth_height_mm = 900;
        document.add_wall(wall);

        let report = validate_structure(&document, &terrain, &style());
        assert!(!report.passed);
        let fix = report
            .findings
            .iter()
            .find_map(|finding| finding.suggested_fix.clone())
            .expect("suggested fix");
        let mut session = Session::from_document("m0a_promontory", terrain.clone(), document);
        session
            .apply_batch(AuthorId::player("mason").unwrap(), vec![fix])
            .unwrap();

        let after = validate_structure(session.document(), &terrain, &style());
        assert!(after.passed, "findings after fix: {:?}", after.findings);
        assert!(after.score > report.score);
        assert!(after
            .findings
            .iter()
            .all(|finding| finding.code != "wall_too_slender"));
    }
}
