//! In-game historical codex.
//!
//! The codex is plain data so it can be reviewed, translated and extended
//! without touching the simulation. Each entry ties a Keepwright design feature
//! to real castles and to published scholarship, giving the player the "why"
//! behind the masonry rules.

use serde::{Deserialize, Serialize};

/// One historical codex entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexEntry {
    /// Stable snake_case identifier used by UI links and save games.
    pub id: String,
    pub title: String,
    pub category: String,
    pub period: String,
    pub summary: String,
    pub body: String,
    pub citations: Vec<String>,
    /// Style pack ids whose rules this entry explains.
    pub related_style_ids: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn entry(
    id: &str,
    title: &str,
    category: &str,
    period: &str,
    summary: &str,
    body: &str,
    citations: &[&str],
    related_style_ids: &[&str],
) -> CodexEntry {
    CodexEntry {
        id: id.to_string(),
        title: title.to_string(),
        category: category.to_string(),
        period: period.to_string(),
        summary: summary.to_string(),
        body: body.to_string(),
        citations: citations.iter().map(|value| value.to_string()).collect(),
        related_style_ids: related_style_ids
            .iter()
            .map(|value| value.to_string())
            .collect(),
    }
}

/// Every codex entry, in a stable reading order.
pub fn list_codex_entries() -> Vec<CodexEntry> {
    vec![
        entry(
            "battered_plinth",
            "Battered Plinth & Talus",
            "Masonry",
            "11th–14th century",
            "A sloping stone skirt at the foot of a wall sheds missiles and braces the footing against mining.",
            "The battered plinth — the talus or escarp — is the outward-sloping band of masonry at the base of a curtain wall or tower. It does three jobs at once: it deflects dropped stones and rubble away from the wall foot, it thickens the base where the wall carries the most load, and it makes the wall harder to mine by forcing sappers to dig beneath an overhanging mass. At Caerphilly and Caernarfon the talus is both structural and expressive, stepping out from the face in a shallow curve that reads as a single deliberate form. Keepwright models the plinth as a distinct projection with its own height and projection so the massing, the course stepping and the plan footprint all agree.",
            &[
                "Caerphilly Castle, Wales (c. 1271), great curtain and talus.",
                "Caernarfon Castle, Wales (1283–1330), Eagle Tower and curtain plinths.",
                "Renn, D. F., Norman Castles in Britain (John Baker, 1973).",
                "Kenyon, J. R., Medieval Fortifications (Leicester University Press, 1990).",
            ],
            &["edwardian_welsh", "baltic_brick"],
        ),
        entry(
            "flanking_towers",
            "Flanking Drum & D-Shaped Towers",
            "Defence",
            "12th–14th century",
            "Projecting towers remove dead ground at the wall foot and let archers sweep every face.",
            "A curtain wall without projections has blind spots at its base, where attackers can work undisturbed. Round and D-shaped towers push the defenders' field of fire forward, so each wall face can be covered from at least two directions. Round drums resist mining and artillery better than square towers because they present no flat face and no corner to lever apart. D-shaped towers combine that curved outer face with a flat inner face, giving more usable room inside the tower and a neat junction with the curtain. The spacing matters as much as the shape: too far apart and the arcs of fire fail to overlap, too close and the towers mask one another. Keepwright's style packs encode both the plan shape and the flanking-distance band.",
            &[
                "Conwy Castle, Wales (1283–1287), eight round towers on a narrow rock.",
                "Caernarfon Castle, Wales (1283–1330), polygonal towers.",
                "Harlech Castle, Wales (1283–1289), concentric ring with round corner towers.",
                "Cathcart King, D. J., The Castle in England and Wales (Croom Helm, 1988).",
            ],
            &["edwardian_welsh"],
        ),
        entry(
            "twin_gatehouse",
            "Twin-Tower Gatehouse & Passage",
            "Defence",
            "13th–14th century",
            "Two flanking drums, a narrow passage and a bridge above turn the gate into the strongest point of the enceinte.",
            "The gate was the weakest and most contested part of any castle, so builders lavished attention on it. The classic Edwardian solution is a twin-tower gatehouse: a clear passage between two powerful drums, with a masonry bridge over the passage carrying a portcullis, murder holes and sometimes a chapel or guard room. Attackers who forced the outer gate found themselves in a narrow corridor overlooked from both sides and from above. At Caernarfon the King's Gate adds a drawbridge and multiple portcullis slots; at Harlech the gatehouse is a self-contained fortress. Keepwright models the gatehouse as its own element so the passage void, flanking drums and bridging mass stay consistent during edits.",
            &[
                "Caernarfon Castle, Wales, King's Gate (1283–1330).",
                "Harlech Castle, Wales, gatehouse (1283–1289).",
                "Beaumaris Castle, Wales, Gate next the Sea and Llanfaes Gate (1295–1330).",
                "Taylor, A. J., Studies in Castles and Castle-Building (Hambledon Press, 1985).",
            ],
            &["edwardian_welsh"],
        ),
        entry(
            "arrow_loops",
            "Splayed Arrow Loops & Embrasures",
            "Defence",
            "12th–15th century",
            "A narrow exterior slit opens into a wide interior embrasure, giving an archer cover and a broad field of fire.",
            "An arrow loop is not a simple hole. From outside it is a thin vertical slit, often with a small horizontal cross for a crossbow; from inside it opens into a wide, splayed embrasure that lets the defender stand back from the wall and shoot across a wide arc. The splay also throws the archer's silhouette into shadow and keeps the bowstring clear of the stone. The sill is set high enough to be awkward to climb but low enough to shoot downward at the wall foot. The geometry is a direct trade-off between protection and field of fire, and it is one of the clearest places where a castle's plan reveals its intended tactics.",
            &[
                "Dover Castle, England, Arthur's Hall and inner bailey loops (12th–13th century).",
                "Caernarfon Castle, Wales, arrow loops in the Eagle Tower and curtain.",
                "Bodiam Castle, England, late 14th-century loops and gun ports.",
                "Kenyon, J. R., Medieval Fortifications (Leicester University Press, 1990).",
            ],
            &["edwardian_welsh", "baltic_brick"],
        ),
        entry(
            "edwardian_welsh",
            "Edwardian Welsh Concentric Castles (1277–1300)",
            "History",
            "1277–1300",
            "Edward I's Welsh castles combine concentric defence, integrated town walls and a single geometric language.",
            "After the conquest of Gwynedd, Edward I and his master mason James of St George built a chain of castles that still define the military architecture of the high Middle Ages. Caernarfon, Conwy, Harlech and Beaumaris are the canonical examples. They share a preference for strong flanking towers, twin-tower gatehouses, concentric lines of defence and close integration with walled towns. The design is not merely defensive: the polygonal towers and banded masonry at Caernarfon deliberately evoke Constantinople and the authority of an imperial past. Keepwright's Edwardian Welsh style pack condenses these rules into numeric guidance for wall thickness, batter, plinth and tower spacing.",
            &[
                "Caernarfon Castle and town walls, Wales (UNESCO World Heritage, 1283–1330).",
                "Conwy Castle and town walls, Wales (UNESCO World Heritage, 1283–1287).",
                "Harlech Castle, Wales (UNESCO World Heritage, 1283–1289).",
                "Beaumaris Castle, Wales (UNESCO World Heritage, 1295–1330).",
                "Taylor, A. J., The Welsh Castles of Edward I (Hambledon Press, 1986).",
            ],
            &["edwardian_welsh"],
        ),
        entry(
            "baltic_brick",
            "Teutonic Baltic Brick Gothic (1280–1410)",
            "History",
            "1280–1410",
            "The Teutonic Order turned fired brick into a monumental defensive language across Prussia and Livonia.",
            "Where the Edwardian castles used dressed stone, the Teutonic Order built in fired brick on a vast scale. Malbork (Marienburg), Toruń and Radzyń Chełmiński show the result: deep red walls, tall slender towers, decorative arcading and a strong preference for repeated vertical bays. Brick demanded different detailing from ashlar. Courses were thinner, walls often thicker, and the material's relative weakness under impact encouraged massive bases, buttresses and carefully bonded corners. The Order's castles were also administrative and commercial centres, so military architecture sat alongside granaries, chapels and refectories. Keepwright's Baltic brick style captures the palette, course rhythm and heavier wall sections of this tradition.",
            &[
                "Malbork Castle, Poland (UNESCO World Heritage, 13th–15th century).",
                "Toruń Castle and town walls, Poland (UNESCO World Heritage, 13th–14th century).",
                "Radzyń Chełmiński Castle, Poland (late 13th–14th century).",
                "Arszyński, M., Budownictwo warowne zakonu krzyżackiego w Prusach (1230–1454) (Toruń, 1995).",
                "Malbork Castle Museum / UNESCO World Heritage documentation.",
            ],
            &["baltic_brick"],
        ),
        entry(
            "keep_and_hall",
            "Great Keep, Hall & Domestic Ranges",
            "Domestic",
            "11th–15th century",
            "A castle was a household as much as a fortress: the keep, hall, chapel and kitchen formed a working residence.",
            "The great keep began as a last refuge, but by the 13th century it often served as the lord's residence, treasury and prison all at once. Its thick walls and small windows read as defensive, yet inside it held a hierarchy of rooms: a ground-floor undercroft for storage, a first-floor hall or solar, and a chapel in a turret or bay. In the courtyard, separate ranges supplied the great hall, chapel and kitchen, each with its own roof pitch and plan. The hall was the public stage for feasting and justice; the chapel served the household's daily offices; the kitchen was a fire-risk building deliberately set apart. Keepwright derives furniture and room labels from these elements so the interior reads as a lived-in household rather than an empty shell.",
            &[
                "Dover Castle, England, great keep (c. 1180s).",
                "Rochester Castle, England, keep (12th–13th century).",
                "Malbork Castle, Poland, Grand Master's Palace and refectory (14th century).",
                "Wood, M., The English Mediaeval House (Ferndale Editions, 1981).",
            ],
            &["edwardian_welsh", "baltic_brick"],
        ),
        entry(
            "moat_and_ditch",
            "Approach Ditches, Moats & Earthworks",
            "Earthworks",
            "11th–15th century",
            "A ditch is cheap, fast and effective: it stops engines, channels attackers and turns the ground itself into a defence.",
            "Earthworks are the oldest and most economical part of a castle. A ditch or moat denies a clean approach to the wall foot, keeps siege engines out of bowshot and forces attackers into prepared killing zones. Where the ground is wet, a moat adds a water obstacle; where it is dry, the ditch and its upcast bank still slow an assault and expose attackers on the glacis. Ditches also drain the site and mark the boundary of the lord's jurisdiction. The profile matters: a flat floor with sloping sides is easy to cut and hard to cross, while a narrow deep cut is cheaper but more easily bridged. Keepwright treats ditches as terrain elements rather than masonry, so the mesh, foundations and validation all sample the same carved ground.",
            &[
                "Caerphilly Castle, Wales, great water moat and earthwork dams (c. 1271).",
                "Bodiam Castle, England, broad moat (late 14th century).",
                "Malbork Castle, Poland, wet moat and outer earthworks (14th century).",
                "Kenyon, J. R., Medieval Fortifications (Leicester University Press, 1990).",
            ],
            &["edwardian_welsh", "baltic_brick"],
        ),
    ]
}

/// Look up one codex entry by its stable id.
pub fn find_codex_entry(id: &str) -> Option<CodexEntry> {
    list_codex_entries()
        .into_iter()
        .find(|entry| entry.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_has_eight_unique_entries_with_citations() {
        let entries = list_codex_entries();
        assert_eq!(entries.len(), 8);
        let mut ids = std::collections::BTreeSet::new();
        for entry in &entries {
            assert!(ids.insert(entry.id.clone()), "duplicate id {}", entry.id);
            assert!(!entry.title.is_empty());
            assert!(!entry.category.is_empty());
            assert!(!entry.period.is_empty());
            assert!(!entry.summary.is_empty());
            assert!(entry.body.len() > 200, "{} body too short", entry.id);
            assert!(!entry.citations.is_empty(), "{} has no citations", entry.id);
            assert!(entry
                .citations
                .iter()
                .all(|citation| !citation.trim().is_empty()));
        }
    }

    #[test]
    fn find_codex_entry_resolves_known_ids_and_rejects_unknown() {
        for id in [
            "battered_plinth",
            "flanking_towers",
            "twin_gatehouse",
            "arrow_loops",
            "edwardian_welsh",
            "baltic_brick",
            "keep_and_hall",
            "moat_and_ditch",
        ] {
            let found = find_codex_entry(id).expect("known entry");
            assert_eq!(found.id, id);
        }
        assert!(find_codex_entry("not_a_real_entry").is_none());
    }
}
