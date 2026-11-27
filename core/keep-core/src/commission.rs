//! Commissions: curated briefs, seasons, frost and seasonal events.
//!
//! A commission turns the free-build sandbox into a finite historical problem:
//! a patron, a budget in silver, a season deadline, a workforce and a set of
//! goals that the structure and the historian must both accept. Everything is
//! deterministic and driven from the castle document, so a commission can be
//! replayed and tested without a renderer.

use serde::{Deserialize, Serialize};

use crate::document::{CastleDocument, Element, RangeKind};
use crate::historian::assess_historian;
use crate::masonry::generate_blocks;
use crate::salvage::SILVER_PER_100_BLOCKS;
use crate::style::StylePack;
use crate::terrain::SiteTerrain;
use crate::validate::validate_structure;

/// The four seasons of the commission calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

impl Season {
    /// The following season, plus whether the year rolled over.
    pub fn next(self) -> (Season, bool) {
        match self {
            Self::Spring => (Self::Summer, false),
            Self::Summer => (Self::Autumn, false),
            Self::Autumn => (Self::Winter, false),
            Self::Winter => (Self::Spring, true),
        }
    }

    /// Winter is the frost season: mortar slows and fresh earthworks slump.
    pub fn is_frost(self) -> bool {
        matches!(self, Self::Winter)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Spring => "Spring",
            Self::Summer => "Summer",
            Self::Autumn => "Autumn",
            Self::Winter => "Winter",
        }
    }
}

/// One deterministic event that may befall a commission in a season.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonalEvent {
    pub id: String,
    pub title: String,
    pub description: String,
    pub silver_delta: i64,
    pub patience_delta: i32,
    pub workforce_delta: i32,
}

/// One commission requirement and whether the current castle meets it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommissionGoal {
    pub id: String,
    pub label: String,
    pub met: bool,
}

/// A curated commission brief.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommissionBrief {
    pub id: String,
    pub title: String,
    pub patron: String,
    pub description: String,
    pub site_id: String,
    pub style_id: String,
    pub starting_silver: i64,
    pub season_limit: u32,
    pub starting_workforce: u32,
    pub min_structure_score: u32,
    pub min_historian_score: u32,
    pub require_gatehouse: bool,
    pub require_keep: bool,
    pub require_hall: bool,
    pub require_chapel: bool,
    pub require_ditch: bool,
    pub min_towers: usize,
    pub min_walls: usize,
}

/// Live state of one commission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommissionState {
    pub brief: CommissionBrief,
    pub year: u32,
    pub season: Season,
    pub seasons_elapsed: u32,
    pub treasury_silver: i64,
    pub spent_silver: i64,
    pub salvage_silver: i64,
    pub workforce: u32,
    pub patron_patience: u32,
    pub frost_active: bool,
    pub walls_thatched: bool,
    pub active_event: Option<SeasonalEvent>,
    pub goals: Vec<CommissionGoal>,
    pub completed: bool,
    pub failed: bool,
    pub status_note: String,
}

/// The starting patience of every patron, in `0..=100`.
pub const STARTING_PATIENCE: u32 = 100;

#[allow(clippy::too_many_arguments)]
fn brief(
    id: &str,
    title: &str,
    patron: &str,
    description: &str,
    site_id: &str,
    style_id: &str,
    starting_silver: i64,
    season_limit: u32,
    starting_workforce: u32,
    min_structure_score: u32,
    min_historian_score: u32,
) -> CommissionBrief {
    CommissionBrief {
        id: id.to_string(),
        title: title.to_string(),
        patron: patron.to_string(),
        description: description.to_string(),
        site_id: site_id.to_string(),
        style_id: style_id.to_string(),
        starting_silver,
        season_limit,
        starting_workforce,
        min_structure_score,
        min_historian_score,
        require_gatehouse: false,
        require_keep: false,
        require_hall: false,
        require_chapel: false,
        require_ditch: false,
        min_towers: 0,
        min_walls: 0,
    }
}

/// Every curated commission, in stable id order.
pub fn list_commissions() -> Vec<CommissionBrief> {
    let mut commissions = vec![
        CommissionBrief {
            require_gatehouse: true,
            min_towers: 2,
            min_walls: 2,
            ..brief(
                "harlech_watch",
                "Harlech Headland Watch",
                "Master James of St George",
                "Raise a watch over the Irish Sea from the headland. The patron wants a gatehouse that commands the landward approach, flanked by at least two drums and two curtain runs, built in the Edwardian Welsh manner.",
                "coastal_headland",
                "edwardian_welsh",
                1_200,
                8,
                40,
                85,
                60,
            )
        },
        CommissionBrief {
            require_keep: true,
            require_hall: true,
            require_chapel: true,
            min_towers: 2,
            min_walls: 2,
            ..brief(
                "malbork_priory",
                "Vistula Island Convent",
                "Komtur Heinrich von Plauen",
                "On the lake island, raise a brick convent-fortress: a tall keep, a hall and a chapel, with at least two drums and two curtain runs. The Order expects the domestic ranges to read as one roofed composition.",
                "lake_island",
                "baltic_brick",
                1_800,
                10,
                60,
                85,
                70,
            )
        },
        CommissionBrief {
            require_ditch: true,
            min_towers: 2,
            min_walls: 2,
            ..brief(
                "promontory_bastion",
                "River Promontory Bastion",
                "Baroness Eleanor de Clare",
                "Hold the river crossing from the promontory. A ditch across the landward neck, two drums and two curtain runs are the minimum her steward will accept, and the stonework must look as though it belongs to the Welsh marches.",
                "m0a_promontory",
                "edwardian_welsh",
                900,
                6,
                30,
                80,
                55,
            )
        },
        CommissionBrief {
            require_keep: true,
            require_hall: true,
            min_towers: 0,
            min_walls: 2,
            ..brief(
                "winter_keep",
                "Northern Frontier Keep",
                "Voigt Konrad of Konigsberg",
                "The frontier chapter house needs a warm brick keep and hall behind two curtain runs before the frost sets in. There is little time and less money, so every dismantled stone must be salvaged.",
                "lake_island",
                "baltic_brick",
                1_100,
                8,
                45,
                85,
                65,
            )
        },
    ];
    commissions.sort_by(|a, b| a.id.cmp(&b.id));
    commissions
}

/// Look up a curated commission by id.
pub fn find_commission(id: &str) -> Option<CommissionBrief> {
    list_commissions().into_iter().find(|brief| brief.id == id)
}

/// Deterministic silver cost of a castle, from its masonry block count.
///
/// `SILVER_PER_100_BLOCKS` is ten, so a 4,700-block Edwardian showcase costs
/// 470 silver and a 12,000-block Baltic showcase costs 1,200 silver. That keeps
/// the 900-1,800 silver commission purses genuinely tight but achievable.
pub fn estimate_castle_cost_silver(doc: &CastleDocument, terrain: &SiteTerrain) -> i64 {
    let ditches: Vec<&crate::document::Ditch> = doc.ditches().collect();
    let sculpts: Vec<&crate::document::TerrainSculpt> = doc.terrain_sculpts().collect();
    let effective = terrain.with_earthworks(&ditches, &sculpts);
    let block_count = generate_blocks(doc, &effective).len() as i64;
    (block_count * SILVER_PER_100_BLOCKS + 99) / 100
}

/// Evaluate every requirement in `brief` against the current document.
pub fn evaluate_commission_goals(
    brief: &CommissionBrief,
    doc: &CastleDocument,
    terrain: &SiteTerrain,
    style: &StylePack,
) -> Vec<CommissionGoal> {
    let ditches: Vec<&crate::document::Ditch> = doc.ditches().collect();
    let sculpts: Vec<&crate::document::TerrainSculpt> = doc.terrain_sculpts().collect();
    let effective = terrain.with_earthworks(&ditches, &sculpts);

    let structure_score = validate_structure(doc, &effective, style).score;
    let historian_score = assess_historian(doc, &effective, style).overall_score;

    let mut walls = 0usize;
    let mut towers = 0usize;
    let mut gatehouses = 0usize;
    let mut keeps = 0usize;
    let mut halls = 0usize;
    let mut chapels = 0usize;
    let mut ditch_count = 0usize;
    for element in doc.elements() {
        match element {
            Element::BatteredWall(_) => walls += 1,
            Element::RoundTower(_) => towers += 1,
            Element::Gatehouse(_) => gatehouses += 1,
            Element::Keep(_) => keeps += 1,
            Element::Range(range) => match range.kind {
                RangeKind::Hall => halls += 1,
                RangeKind::Chapel => chapels += 1,
                RangeKind::Kitchen => {}
            },
            Element::Ditch(_) => ditch_count += 1,
            Element::TerrainSculpt(_) => {}
        }
    }

    let mut goals = Vec::new();
    let mut push = |id: &str, label: String, met: bool| {
        goals.push(CommissionGoal {
            id: id.to_string(),
            label,
            met,
        });
    };

    push(
        "structure_score",
        format!("Structure score >= {}", brief.min_structure_score),
        structure_score >= brief.min_structure_score,
    );
    push(
        "historian_score",
        format!("Historian score >= {}", brief.min_historian_score),
        historian_score >= brief.min_historian_score,
    );
    if brief.require_gatehouse {
        push(
            "gatehouse",
            "Gatehouse commanding the approach".to_string(),
            gatehouses > 0,
        );
    }
    if brief.require_keep {
        push("keep", "A great keep".to_string(), keeps > 0);
    }
    if brief.require_hall {
        push("hall", "A hall range".to_string(), halls > 0);
    }
    if brief.require_chapel {
        push("chapel", "A chapel range".to_string(), chapels > 0);
    }
    if brief.require_ditch {
        push(
            "ditch",
            "A ditch across the approach".to_string(),
            ditch_count > 0,
        );
    }
    if brief.min_towers > 0 {
        push(
            "towers",
            format!("At least {} towers", brief.min_towers),
            towers >= brief.min_towers,
        );
    }
    if brief.min_walls > 0 {
        push(
            "walls",
            format!("At least {} curtain walls", brief.min_walls),
            walls >= brief.min_walls,
        );
    }

    goals
}

/// Deterministic seasonal event for a commission and elapsed season.
///
/// The same commission at the same elapsed season always gets the same event,
/// which keeps replays and tests stable while still giving each play-through a
/// varied calendar.
pub fn deterministic_seasonal_event(
    commission_id: &str,
    seasons_elapsed: u32,
    season: Season,
) -> SeasonalEvent {
    let seed = crate::fnv1a_64(commission_id.as_bytes());
    let season_ord = match season {
        Season::Spring => 0,
        Season::Summer => 1,
        Season::Autumn => 2,
        Season::Winter => 3,
    };

    let events: &[(&str, &str, &str, i64, i32, i32)] = match season {
        Season::Winter => &[
            (
                "hard_winter",
                "Hard Winter",
                "Frost closes the quarry and sickens the mortar gangs. Work slows and the patron's patience wears thin.",
                -25,
                -6,
                -4,
            ),
            (
                "long_frost",
                "Long Frost",
                "The ground stays frozen into the new year. Haulage stops and the masons huddle by the fire.",
                -10,
                -2,
                -6,
            ),
            (
                "calm_season",
                "A Quiet Winter",
                "The frost is mild and the household settles in without incident.",
                0,
                0,
                0,
            ),
        ],
        Season::Spring => &[
            (
                "quarry_flood",
                "Quarry Flood",
                "Spring melt floods the quarry road. Stone arrives late and the steward demands compensation.",
                -35,
                -5,
                -2,
            ),
            (
                "royal_grant",
                "Royal Grant",
                "A royal warrant brings a modest subsidy and fresh goodwill to the works.",
                120,
                5,
                3,
            ),
            (
                "calm_season",
                "A Calm Spring",
                "Fine building weather and no news from the court.",
                0,
                0,
                0,
            ),
        ],
        Season::Summer => &[
            (
                "master_masons_arrive",
                "Master Masons Arrive",
                "A guild of travelling masons offers its services for the season, for a fee.",
                -60,
                8,
                6,
            ),
            (
                "royal_grant",
                "Royal Grant",
                "The patron's favour at court is rewarded with silver for the works.",
                120,
                5,
                3,
            ),
            (
                "calm_season",
                "A Long Summer",
                "Warm dry weather and an uneventful season on the works.",
                0,
                0,
                0,
            ),
        ],
        Season::Autumn => &[
            (
                "war_tax",
                "War Tax",
                "The crown levies a tax for a distant campaign. The treasury feels it immediately.",
                -80,
                -4,
                -3,
            ),
            (
                "master_masons_arrive",
                "Master Masons Arrive",
                "A guild of travelling masons offers its services before the frost, for a fee.",
                -50,
                4,
                4,
            ),
            (
                "royal_grant",
                "Royal Grant",
                "A timely grant refills the works' purse before winter.",
                120,
                5,
                3,
            ),
            (
                "calm_season",
                "A Quiet Autumn",
                "The harvest is good and the works proceed without interruption.",
                0,
                0,
                0,
            ),
        ],
    };

    let roll = crate::hash01(seed, i64::from(seasons_elapsed), i64::from(season_ord), 31);
    let index = ((roll * events.len() as f32) as usize).min(events.len() - 1);
    let (id, title, description, silver_delta, patience_delta, workforce_delta) = events[index];
    SeasonalEvent {
        id: id.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        silver_delta,
        patience_delta,
        workforce_delta,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_curated_commissions_are_listed_in_stable_order() {
        let commissions = list_commissions();
        assert_eq!(commissions.len(), 4);
        let ids: Vec<&str> = commissions.iter().map(|brief| brief.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "harlech_watch",
                "malbork_priory",
                "promontory_bastion",
                "winter_keep"
            ]
        );
    }

    #[test]
    fn every_commission_points_at_a_real_site_and_style() {
        for brief in list_commissions() {
            assert!(
                SiteTerrain::for_site(&brief.site_id).is_some(),
                "{} has unknown site {}",
                brief.id,
                brief.site_id
            );
            assert!(
                StylePack::find(&brief.style_id).is_some(),
                "{} has unknown style {}",
                brief.id,
                brief.style_id
            );
            assert!(brief.starting_silver > 0, "{} is unfunded", brief.id);
            assert!(brief.season_limit > 0, "{} has no deadline", brief.id);
            assert!(find_commission(&brief.id).is_some());
        }
        assert!(find_commission("does_not_exist").is_none());
    }

    #[test]
    fn season_cycle_rolls_the_year_after_winter() {
        assert_eq!(Season::Spring.next(), (Season::Summer, false));
        assert_eq!(Season::Summer.next(), (Season::Autumn, false));
        assert_eq!(Season::Autumn.next(), (Season::Winter, false));
        assert_eq!(Season::Winter.next(), (Season::Spring, true));
        assert!(Season::Winter.is_frost());
        assert!(!Season::Spring.is_frost());
        assert_eq!(Season::Autumn.label(), "Autumn");
    }

    #[test]
    fn seasonal_events_are_deterministic() {
        let first = deterministic_seasonal_event("promontory_bastion", 3, Season::Winter);
        let second = deterministic_seasonal_event("promontory_bastion", 3, Season::Winter);
        assert_eq!(first, second);
        assert!(!first.id.is_empty());
        assert!(!first.title.is_empty());

        let other = deterministic_seasonal_event("promontory_bastion", 2, Season::Summer);
        assert_ne!(first.id, other.id);
    }

    #[test]
    fn showcase_costs_are_affordable_within_the_commission_purses() {
        let m1 = crate::fixtures::m1_edwardian_document();
        let m1_cost =
            estimate_castle_cost_silver(&m1, &SiteTerrain::for_site("coastal_headland").unwrap());
        let harlech = find_commission("harlech_watch").unwrap();
        assert!(
            m1_cost > 0 && m1_cost < harlech.starting_silver,
            "M1 cost {m1_cost} should fit inside {} silver",
            harlech.starting_silver
        );

        let m2 = crate::fixtures::m2_baltic_document();
        let m2_cost =
            estimate_castle_cost_silver(&m2, &SiteTerrain::for_site("lake_island").unwrap());
        let malbork = find_commission("malbork_priory").unwrap();
        assert!(
            m2_cost > 0 && m2_cost < malbork.starting_silver,
            "M2 cost {m2_cost} should fit inside {} silver",
            malbork.starting_silver
        );
    }

    #[test]
    fn an_empty_document_meets_no_goals() {
        let brief = find_commission("promontory_bastion").unwrap();
        let terrain = SiteTerrain::for_site(&brief.site_id).unwrap();
        let style = StylePack::find(&brief.style_id).unwrap();
        let goals = evaluate_commission_goals(&brief, &CastleDocument::new(), &terrain, &style);
        assert!(!goals.is_empty());
        assert!(goals.iter().any(|goal| !goal.met));
        assert!(goals
            .iter()
            .find(|goal| goal.id == "historian_score")
            .is_some_and(|goal| !goal.met));
    }
}
