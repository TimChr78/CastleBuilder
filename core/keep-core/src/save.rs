//! Versioned `castle.keep.json` save format.
//!
//! The save file carries both the *initial* document and the append-only command
//! log. Loading replays the log from the initial document and verifies the
//! resulting document hash, which catches hand-edited or corrupted saves
//! cleanly instead of silently producing a different castle (architecture
//! section 4.2).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::command::{AuthorId, CommandError, CommandPayload, LoggedCommand};
use crate::document::CastleDocument;
use crate::session::{Session, UndoError, DEFAULT_SITE_ID};
use crate::terrain::SiteTerrain;
use crate::GENERATOR_VERSION;

/// Current on-disk save schema version.
pub const SAVE_SCHEMA_VERSION: u32 = 1;

/// Largest accepted save file, in bytes (8 MiB). Protects the loader from
/// pathological JSON before any of it is parsed.
pub const MAX_SAVE_BYTES: usize = 8 * 1024 * 1024;

/// Largest accepted command log. Replay is linear but each entry still costs
/// validation time, so bound the untrusted history.
pub const MAX_SAVE_COMMANDS: usize = 10_000;

/// Largest accepted site id, in characters.
pub const MAX_SITE_ID_CHARS: usize = 64;

/// Site ids this build can reconstruct terrain for. Unknown sites are rejected
/// rather than silently falling back to a different landscape.
pub const KNOWN_SITE_IDS: &[&str] = &[DEFAULT_SITE_ID, "coastal_headland", "lake_island"];

/// The JSON shape of `castle.keep.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SaveFile {
    pub schema_version: u32,
    pub generator_version: u32,
    pub site_id: String,
    /// Document before `command_log` was applied.
    pub initial_document: CastleDocument,
    /// Document after applying `command_log` (cached for fast loads).
    pub document: CastleDocument,
    pub command_log: Vec<LoggedCommand>,
    pub next_element_id: u64,
    /// Hash of `document`; verified again after replay.
    pub document_hash: u64,
}

/// Why a save file could not be written or loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveError {
    Serialize(String),
    Deserialize(String),
    UnsupportedSchemaVersion {
        found: u32,
        expected: u32,
    },
    /// The cached `document` does not hash to the stored `document_hash`.
    CorruptedDocumentHash {
        stored: u64,
        actual: u64,
    },
    /// Replaying the log did not reproduce the saved document.
    ReplayMismatch {
        expected: u64,
        replayed: u64,
    },
    /// The id allocator would reuse an existing id.
    InvalidNextElementId {
        next: u64,
        max: u64,
    },
    /// The save file exceeds [`MAX_SAVE_BYTES`].
    SaveTooLarge {
        bytes: usize,
        max: usize,
    },
    /// The save file carries more than [`MAX_SAVE_COMMANDS`] entries.
    TooManyCommands {
        count: usize,
        max: usize,
    },
    /// The save was produced by a different geometry generator.
    UnsupportedGeneratorVersion {
        found: u32,
        expected: u32,
    },
    /// The save names a site this build cannot reconstruct.
    UnknownSiteId {
        site_id: String,
    },
    /// Command log entries must be contiguous and start at sequence 1.
    InvalidCommandSequence {
        expected: u64,
        found: u64,
    },
    /// An empty batch must never be stored in a save (loading would replay it
    /// as a no-op and quietly drop an entry).
    EmptyBatchCommand {
        seq: u64,
    },
    /// A compensating undo named a different target than the one the real undo
    /// API selected.
    UndoTargetMismatch {
        expected: u64,
        found: u64,
    },
    /// Replaying a compensating undo failed (for example because the target was
    /// not the author's latest live batch).
    Undo(UndoError),
    Command(CommandError),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serialize(message) => write!(f, "could not serialise save: {message}"),
            Self::Deserialize(message) => write!(f, "could not parse save: {message}"),
            Self::UnsupportedSchemaVersion { found, expected } => write!(
                f,
                "unsupported save schema version {found}; expected {expected}"
            ),
            Self::CorruptedDocumentHash { stored, actual } => write!(
                f,
                "save document hash mismatch: stored {stored}, actual {actual}"
            ),
            Self::ReplayMismatch { expected, replayed } => write!(
                f,
                "command log replayed to hash {replayed}, expected {expected}"
            ),
            Self::InvalidNextElementId { next, max } => write!(
                f,
                "next_element_id {next} would reuse an existing id (highest is {max})"
            ),
            Self::SaveTooLarge { bytes, max } => {
                write!(f, "save is {bytes} bytes; maximum is {max}")
            }
            Self::TooManyCommands { count, max } => {
                write!(f, "command log has {count} entries; maximum is {max}")
            }
            Self::UnsupportedGeneratorVersion { found, expected } => write!(
                f,
                "unsupported generator version {found}; expected {expected}"
            ),
            Self::UnknownSiteId { site_id } => write!(f, "unknown site id '{site_id}'"),
            Self::InvalidCommandSequence { expected, found } => write!(
                f,
                "command log sequence is not contiguous: expected {expected}, found {found}"
            ),
            Self::EmptyBatchCommand { seq } => {
                write!(f, "command log entry {seq} is an empty batch")
            }
            Self::UndoTargetMismatch { expected, found } => write!(
                f,
                "compensating undo targets command {found}, but the real undo API selected {expected}"
            ),
            Self::Undo(error) => write!(f, "{error}"),
            Self::Command(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for SaveError {}

impl From<CommandError> for SaveError {
    fn from(error: CommandError) -> Self {
        Self::Command(error)
    }
}

impl From<UndoError> for SaveError {
    fn from(error: UndoError) -> Self {
        Self::Undo(error)
    }
}

impl Session {
    /// Serialise the session as pretty-printed `castle.keep.json`.
    pub fn to_save_json(&self) -> Result<String, SaveError> {
        let save = SaveFile {
            schema_version: SAVE_SCHEMA_VERSION,
            generator_version: GENERATOR_VERSION,
            site_id: self.site_id.clone(),
            initial_document: self.initial_document.clone(),
            document: self.document.clone(),
            command_log: self.command_log.clone(),
            next_element_id: self.next_element_id,
            document_hash: self.document.document_hash(),
        };
        serde_json::to_string_pretty(&save).map_err(|error| SaveError::Serialize(error.to_string()))
    }

    /// Deserialise and verify a `castle.keep.json`, replaying the command log
    /// from the initial document before trusting the cached document.
    pub fn from_save_json(json: &str) -> Result<Session, SaveError> {
        if json.len() > MAX_SAVE_BYTES {
            return Err(SaveError::SaveTooLarge {
                bytes: json.len(),
                max: MAX_SAVE_BYTES,
            });
        }

        let save: SaveFile = serde_json::from_str(json)
            .map_err(|error| SaveError::Deserialize(error.to_string()))?;
        if save.schema_version != SAVE_SCHEMA_VERSION {
            return Err(SaveError::UnsupportedSchemaVersion {
                found: save.schema_version,
                expected: SAVE_SCHEMA_VERSION,
            });
        }
        if save.generator_version != GENERATOR_VERSION {
            return Err(SaveError::UnsupportedGeneratorVersion {
                found: save.generator_version,
                expected: GENERATOR_VERSION,
            });
        }
        validate_site_id(&save.site_id)?;
        if save.command_log.len() > MAX_SAVE_COMMANDS {
            return Err(SaveError::TooManyCommands {
                count: save.command_log.len(),
                max: MAX_SAVE_COMMANDS,
            });
        }

        // The initial document is untrusted input: reject forged map keys,
        // dangling openings and out-of-bounds geometry before replaying (H3).
        save.initial_document.validate()?;

        // The cached document is only trusted as a hash commitment; the real
        // authority is the freshly replayed document.
        let actual = save.document.document_hash();
        if actual != save.document_hash {
            return Err(SaveError::CorruptedDocumentHash {
                stored: save.document_hash,
                actual,
            });
        }

        let terrain = terrain_for_site(&save.site_id);
        let mut replayed =
            Session::from_document(save.site_id.clone(), terrain, save.initial_document.clone());

        // Replay every entry through the real Session API. This discards all
        // untrusted `inverse`, `touched_elements`, `summary` and `command_id`
        // fields and regenerates them from the validated operations (H1/H2).
        for (index, entry) in save.command_log.iter().enumerate() {
            let expected_seq = (index as u64) + 1;
            if entry.seq != expected_seq {
                return Err(SaveError::InvalidCommandSequence {
                    expected: expected_seq,
                    found: entry.seq,
                });
            }
            // Re-sanitise the author id: control and bidi characters must not
            // survive a round trip through the save file.
            let author = AuthorId::new(entry.author.kind, entry.author.id.clone())?;
            match &entry.payload {
                CommandPayload::Batch { ops } => {
                    // Loading must reproduce the exact same sequence numbers,
                    // so an empty batch (which apply_batch treats as a no-op
                    // and does not log) is structurally invalid in a save.
                    if ops.is_empty() {
                        return Err(SaveError::EmptyBatchCommand { seq: entry.seq });
                    }
                    replayed.apply_batch(author, ops.clone())?;
                }
                CommandPayload::CompensateUndo { target_seq } => {
                    let outcome = replayed.undo_last(&author)?;
                    if outcome.target_seq != *target_seq {
                        return Err(SaveError::UndoTargetMismatch {
                            expected: outcome.target_seq,
                            found: *target_seq,
                        });
                    }
                }
            }
        }

        // Reject an allocator that would reuse an id from deleted history.
        if save.next_element_id < replayed.next_element_id {
            return Err(SaveError::InvalidNextElementId {
                next: save.next_element_id,
                max: replayed.next_element_id.saturating_sub(1),
            });
        }

        let replayed_hash = replayed.document.document_hash();
        if replayed_hash != save.document_hash {
            return Err(SaveError::ReplayMismatch {
                expected: save.document_hash,
                replayed: replayed_hash,
            });
        }

        // Keep the on-disk allocator (which may be ahead of replay) and the
        // regenerated command log. `replayed` already carries next_seq and
        // clock_ms consistent with DEFAULT_CLOCK_MS + seq (L1).
        replayed.next_element_id = save.next_element_id;
        Ok(replayed)
    }
}

/// Reconstruct terrain authority from the site id stored in the save. Callers
/// must validate the id with [`validate_site_id`] first; unknown sites are
/// rejected rather than silently mapped onto a different landscape.
pub fn terrain_for_site(site_id: &str) -> SiteTerrain {
    SiteTerrain::for_site(site_id).unwrap_or_else(SiteTerrain::m0a_promontory)
}

/// Reject unknown or oversized site ids.
fn validate_site_id(site_id: &str) -> Result<(), SaveError> {
    let chars = site_id.chars().count();
    let known = KNOWN_SITE_IDS.contains(&site_id);
    if !known || chars == 0 || chars > MAX_SITE_ID_CHARS {
        return Err(SaveError::UnknownSiteId {
            site_id: site_id.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{AuthorId, CommandOp};
    use crate::fixtures::m0b_showcase_document;
    use crate::session::DEFAULT_SITE_ID;

    fn player() -> AuthorId {
        AuthorId::player("alex").unwrap()
    }

    fn agent() -> AuthorId {
        AuthorId::agent("mason").unwrap()
    }

    fn session_with_history() -> Session {
        let mut session = Session::from_document(
            DEFAULT_SITE_ID,
            SiteTerrain::m0a_promontory(),
            m0b_showcase_document(),
        );
        session
            .apply_batch(
                player(),
                vec![
                    CommandOp::CreateWallRun {
                        ref_id: Some("south".into()),
                        name: "South curtain".into(),
                        start_mm: [-6_000, 22_000],
                        end_mm: [26_000, 22_000],
                        height_mm: 8_500,
                        thickness_mm: 2_300,
                        batter_mm: Some(700),
                    },
                    CommandOp::AddArrowLoop {
                        wall_id: None,
                        wall_ref: Some("south".into()),
                        t_permille: 400,
                        sill_height_mm: 1_900,
                        height_mm: None,
                        outer_slit_mm: None,
                        inner_splay_width_mm: None,
                    },
                ],
            )
            .unwrap();
        session
            .apply_batch(
                AuthorId::agent("mason").unwrap(),
                vec![CommandOp::CreateTower {
                    ref_id: None,
                    name: "South east drum".into(),
                    // Deliberately clear of the south curtain so the player's
                    // undo below is not blocked by a shared junction (H4).
                    centre_mm: [46_000, 46_000],
                    radius_mm: 4_500,
                    height_mm: 16_000,
                    batter_mm: None,
                }],
            )
            .unwrap();
        session.undo_last(&player()).unwrap();
        session
    }

    #[test]
    fn save_file_round_trips_through_json() {
        let session = session_with_history();
        let json = session.to_save_json().unwrap();
        assert!(json.contains("\"schema_version\": 1"));
        assert!(json.contains("\"command_log\""));

        let loaded = Session::from_save_json(&json).unwrap();
        assert_eq!(loaded.site_id(), session.site_id());
        assert_eq!(loaded.document(), session.document());
        assert_eq!(loaded.history(), session.history());
        assert_eq!(loaded.next_element_id(), session.next_element_id());
        assert_eq!(
            loaded.document().document_hash(),
            session.document().document_hash()
        );
        assert_eq!(
            loaded.build_scene().layout_hash,
            session.build_scene().layout_hash
        );
    }

    #[test]
    fn save_file_is_deterministic() {
        let session = session_with_history();
        assert_eq!(
            session.to_save_json().unwrap(),
            session.to_save_json().unwrap()
        );
    }

    #[test]
    fn corrupted_document_hash_is_rejected() {
        let session = session_with_history();
        let json = session.to_save_json().unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value["document_hash"] = serde_json::json!(value["document_hash"].as_u64().unwrap() ^ 1);
        let error = Session::from_save_json(&serde_json::to_string(&value).unwrap()).unwrap_err();
        assert!(matches!(error, SaveError::CorruptedDocumentHash { .. }));
    }

    #[test]
    fn tampered_document_with_matching_stored_hash_still_fails_replay() {
        let session = session_with_history();
        let json = session.to_save_json().unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        // Change the cached document and update the stored hash to match it, so
        // only replay can catch the inconsistency.
        let elements = value["document"]["elements"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        value["document"]["elements"][elements.as_str()]["BatteredWall"]["height_mm"] =
            serde_json::json!(1_234);
        let tampered: SaveFile = serde_json::from_value(value.clone()).unwrap();
        value["document_hash"] = serde_json::json!(tampered.document.document_hash());
        let error = Session::from_save_json(&serde_json::to_string(&value).unwrap()).unwrap_err();
        assert!(matches!(error, SaveError::ReplayMismatch { .. }));
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let session = session_with_history();
        let json = session.to_save_json().unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value["schema_version"] = serde_json::json!(2);
        let error = Session::from_save_json(&serde_json::to_string(&value).unwrap()).unwrap_err();
        assert_eq!(
            error,
            SaveError::UnsupportedSchemaVersion {
                found: 2,
                expected: SAVE_SCHEMA_VERSION
            }
        );
    }

    #[test]
    fn malformed_json_is_rejected() {
        assert!(matches!(
            Session::from_save_json("{not json"),
            Err(SaveError::Deserialize(_))
        ));
    }

    #[test]
    fn unused_initial_document_is_preserved() {
        let session = session_with_history();
        let json = session.to_save_json().unwrap();
        let loaded = Session::from_save_json(&json).unwrap();
        assert_eq!(loaded.initial_document, session.initial_document);
    }

    fn wall(name: &str, start: [i64; 2], end: [i64; 2]) -> CommandOp {
        CommandOp::CreateWallRun {
            ref_id: None,
            name: name.into(),
            start_mm: start,
            end_mm: end,
            height_mm: 9_000,
            thickness_mm: 2_400,
            batter_mm: None,
        }
    }

    fn loop_op(wall_id: u64) -> CommandOp {
        CommandOp::AddArrowLoop {
            wall_id: Some(wall_id),
            wall_ref: None,
            t_permille: 500,
            sill_height_mm: 1_800,
            height_mm: None,
            outer_slit_mm: None,
            inner_splay_width_mm: None,
        }
    }

    #[test]
    fn crafted_empty_batch_with_forged_inverse_is_rejected() {
        let mut session = Session::new_empty();
        session
            .apply_batch(player(), vec![wall("North", [0, 0], [10_000, 0])])
            .unwrap();
        let json = session.to_save_json().unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Replace the real batch with an empty one and plant a forged inverse
        // trying to restore a 200 m wall with a bidi name.
        let forged = crate::document::Element::BatteredWall(crate::document::BatteredWall::new(
            99,
            "evil \u{202E} wall",
            [0, 0],
            [200_000, 0],
            9_000,
            2_400,
        ));
        value["command_log"][0]["payload"] = serde_json::json!({
            "kind": "batch",
            "ops": [],
        });
        value["command_log"][0]["inverse"] = serde_json::json!({
            "created_elements": [],
            "created_openings": [],
            "deleted_elements": [forged],
            "deleted_openings": [],
        });

        let error = Session::from_save_json(&serde_json::to_string(&value).unwrap()).unwrap_err();
        assert!(
            matches!(error, SaveError::EmptyBatchCommand { seq: 1 }),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn forged_touched_elements_do_not_weaken_junction_undo_protection() {
        let mut session = Session::new_empty();
        session
            .apply_batch(player(), vec![wall("North", [0, 0], [10_000, 0])])
            .unwrap();
        session.apply_batch(agent(), vec![loop_op(1)]).unwrap();

        let json = session.to_save_json().unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        // The agent erases its footprint from the save's metadata.
        value["command_log"][1]["touched_elements"] = serde_json::json!([]);
        value["command_log"][1]["junction_elements"] = serde_json::json!([]);

        let mut loaded = Session::from_save_json(&serde_json::to_string(&value).unwrap()).unwrap();
        // Replay regenerated the real footprint, so the player's undo is still
        // (correctly) refused.
        assert!(matches!(
            loaded.undo_last(&player()),
            Err(UndoError::BlockedByLaterCommand {
                target_seq: 1,
                blocking_seq: 2,
                ..
            })
        ));
    }

    #[test]
    fn crafted_initial_document_is_validated_before_replay() {
        let mut doc = CastleDocument::new();
        doc.add_wall(crate::document::BatteredWall::new(
            1,
            "North",
            [0, 0],
            [10_000, 0],
            9_000,
            2_400,
        ));
        let session = Session::from_document(DEFAULT_SITE_ID, SiteTerrain::m0a_promontory(), doc);
        let json = session.to_save_json().unwrap();

        // 1. Map key that disagrees with the embedded id.
        let mut mismatched: serde_json::Value = serde_json::from_str(&json).unwrap();
        let elements = mismatched["initial_document"]["elements"]
            .as_object_mut()
            .unwrap();
        let element = elements.remove("1").unwrap();
        elements.insert("2".into(), element);
        let error =
            Session::from_save_json(&serde_json::to_string(&mismatched).unwrap()).unwrap_err();
        assert!(
            matches!(
                error,
                SaveError::Command(CommandError::ElementIdMismatch { key: 2, actual: 1 })
            ),
            "unexpected error: {error:?}"
        );

        // 2. Negative wall thickness.
        let mut negative: serde_json::Value = serde_json::from_str(&json).unwrap();
        negative["initial_document"]["elements"]["1"]["BatteredWall"]["thickness_mm"] =
            serde_json::json!(-100);
        let error =
            Session::from_save_json(&serde_json::to_string(&negative).unwrap()).unwrap_err();
        assert!(
            matches!(
                error,
                SaveError::Command(CommandError::DimensionOutOfRange {
                    field: "thickness_mm",
                    ..
                })
            ),
            "unexpected error: {error:?}"
        );

        // 3. Opening whose wall id dangles.
        let mut dangling: serde_json::Value = serde_json::from_str(&json).unwrap();
        let opening = crate::document::Opening::arrow_loop(10, 999, "loop", 500, 1_500);
        dangling["initial_document"]["openings"] = serde_json::json!({ "10": opening });
        let error =
            Session::from_save_json(&serde_json::to_string(&dangling).unwrap()).unwrap_err();
        assert!(
            matches!(error, SaveError::Command(CommandError::UnknownWallId(999))),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn crafted_next_element_id_that_reuses_deleted_history_is_rejected() {
        let mut session = Session::new_empty();
        session
            .apply_batch(player(), vec![wall("North", [0, 0], [10_000, 0])])
            .unwrap();
        session
            .apply_batch(player(), vec![CommandOp::DeleteElement { id: 1 }])
            .unwrap();
        let json = session.to_save_json().unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value["next_element_id"] = serde_json::json!(1);

        let error = Session::from_save_json(&serde_json::to_string(&value).unwrap()).unwrap_err();
        assert!(
            matches!(error, SaveError::InvalidNextElementId { next: 1, max: 1 }),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn crafted_self_targeting_undo_is_rejected() {
        let mut session = Session::new_empty();
        session
            .apply_batch(player(), vec![wall("North", [0, 0], [10_000, 0])])
            .unwrap();
        let json = session.to_save_json().unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();

        let mut forged = value["command_log"][0].clone();
        forged["seq"] = serde_json::json!(2);
        forged["payload"] = serde_json::json!({
            "kind": "compensate_undo",
            "target_seq": 2,
        });
        forged["touched_elements"] = serde_json::json!([]);
        forged["junction_elements"] = serde_json::json!([]);
        value["command_log"] = serde_json::json!([value["command_log"][0].clone(), forged]);

        let error = Session::from_save_json(&serde_json::to_string(&value).unwrap()).unwrap_err();
        assert!(
            matches!(
                error,
                SaveError::UndoTargetMismatch {
                    expected: 1,
                    found: 2
                }
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn wrong_generator_version_and_unknown_site_are_rejected() {
        let session = session_with_history();
        let json = session.to_save_json().unwrap();

        let mut wrong_generator: serde_json::Value = serde_json::from_str(&json).unwrap();
        wrong_generator["generator_version"] = serde_json::json!(GENERATOR_VERSION + 1);
        let error =
            Session::from_save_json(&serde_json::to_string(&wrong_generator).unwrap()).unwrap_err();
        assert!(
            matches!(error, SaveError::UnsupportedGeneratorVersion { .. }),
            "unexpected error: {error:?}"
        );

        let mut unknown_site: serde_json::Value = serde_json::from_str(&json).unwrap();
        unknown_site["site_id"] = serde_json::json!("not_a_real_site");
        let error =
            Session::from_save_json(&serde_json::to_string(&unknown_site).unwrap()).unwrap_err();
        assert!(
            matches!(error, SaveError::UnknownSiteId { .. }),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn all_three_curated_sites_round_trip_through_save() {
        assert_eq!(KNOWN_SITE_IDS.len(), 3);
        for site_id in KNOWN_SITE_IDS {
            let terrain = SiteTerrain::for_site(site_id).expect("known site");
            let session = Session::from_document(*site_id, terrain, CastleDocument::new());
            let json = session.to_save_json().unwrap();
            let loaded = Session::from_save_json(&json).unwrap();
            assert_eq!(loaded.site_id(), *site_id);
            assert_eq!(
                loaded.terrain().height_mm(0, 0),
                session.terrain().height_mm(0, 0)
            );
        }
    }
}
