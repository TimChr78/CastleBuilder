//! Co-play protocol types, snapshots and divergence checks (architecture
//! section 15, Release 2 M5 Wave 1).
//!
//! Co-play is host-authoritative: only commands travel, and every client
//! regenerates layout locally from the Castle Document. Two hashes are
//! exchanged so a mismatch can be classified:
//!
//! * a **document hash** mismatch means a missed or corrupted command, so the
//!   host resends the affected element state;
//! * a **layout hash** mismatch with identical documents means generator
//!   divergence, a bug that resending state cannot fix.
//!
//! Joining clients must match the host's `generator_version`, site and style
//! pack hash before they receive a [`CoplaySnapshot`].

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::command::{AuthorId, CommandError, CommandOp};
use crate::document::{CastleDocument, ElementId};
use crate::style::StylePack;

/// Version of the co-play protocol and its snapshot shape. Bumped whenever the
/// wire format or the meaning of a snapshot changes.
pub const GENERATOR_VERSION: u32 = 1;

/// Default player limit: two players plus agents is the design target
/// (architecture section 15.4).
pub const DEFAULT_MAX_PLAYERS: usize = 2;

/// Hard ceiling for the host setting. Four is the stretch goal.
pub const MAX_PLAYERS_LIMIT: usize = 4;

/// What a connected player is allowed to do (architecture section 15.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerRole {
    /// Owns the session and may release anyone's soft claims.
    Host,
    /// May send commands.
    Builder,
    /// May look but not edit.
    Viewer,
}

/// One connected player and their last known presence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerParticipant {
    pub id: String,
    pub name: String,
    pub color_hex: String,
    pub role: PlayerRole,
    /// Camera eye position, integer millimetres.
    pub camera_pos_mm: [i64; 3],
    /// Camera look-at target, integer millimetres.
    pub camera_target_mm: [i64; 3],
    /// Free-form status line ("working on the barbican").
    pub status: String,
}

/// A non-blocking claim on an element. Others are warned, never blocked
/// (architecture section 15.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftClaim {
    pub element_id: ElementId,
    pub owner: AuthorId,
    pub note: String,
}

/// What a joining client must agree with the host before it may join.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinHandshake {
    pub generator_version: u32,
    pub site_id: String,
    pub style_id: String,
    pub style_pack_hash: String,
    pub max_players: usize,
}

/// Everything a joining client needs to reconstruct the session state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoplaySnapshot {
    pub handshake: JoinHandshake,
    /// The castle document. Serialised as element/opening arrays rather than
    /// id-keyed maps so snapshots survive transport inside a
    /// `#[serde(tag = "type")]` message (serde's internally tagged enums
    /// buffer map keys as strings, which would break `u64` keyed maps).
    #[serde(with = "document_serde")]
    pub document: CastleDocument,
    pub next_element_id: ElementId,
    pub next_seq: u64,
    pub players: Vec<PlayerParticipant>,
    pub claims: Vec<SoftClaim>,
    /// Hex document hash for the captured document.
    pub document_hash: String,
    /// Hex generated-layout hash for the captured document.
    pub layout_hash: String,
}

/// Outcome of comparing the host's and client's hashes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HashCheckResult {
    Match,
    /// Documents differ: a command was missed or corrupted.
    DocumentMismatch {
        host_doc_hash: String,
        client_doc_hash: String,
    },
    /// Documents agree but generated layout differs: generator divergence.
    GeneratorDivergence {
        doc_hash: String,
        host_layout_hash: String,
        client_layout_hash: String,
    },
}

/// One co-play wire message. Only commands and presence travel; layout and
/// meshes are always regenerated locally.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoplayMessage {
    /// Client asks to join and announces itself.
    JoinRequest {
        handshake: JoinHandshake,
        player: PlayerParticipant,
    },
    /// Host admits the client with a full state snapshot.
    JoinAccepted {
        handshake: JoinHandshake,
        snapshot: CoplaySnapshot,
    },
    /// Host refuses the join, with a human-readable reason.
    JoinRejected { reason: String },
    /// Client submits a batch for host validation.
    SubmitBatch {
        author: AuthorId,
        ops: Vec<CommandOp>,
    },
    /// Client asks the host to undo its latest live batch.
    SubmitUndo { author: AuthorId },
    /// Host broadcasts an accepted batch in sequence order.
    BroadcastBatch {
        seq: u64,
        command_id: String,
        author: AuthorId,
        ops: Vec<CommandOp>,
        document_hash: String,
    },
    /// Host broadcasts a compensating undo command.
    BroadcastUndo {
        seq: u64,
        command_id: String,
        author: AuthorId,
        target_seq: u64,
        document_hash: String,
    },
    /// Throttled presence update (~10 Hz) from any participant.
    PresenceUpdate {
        player_id: String,
        camera_pos_mm: [i64; 3],
        camera_target_mm: [i64; 3],
        status: String,
    },
    /// Set (`Some`) or release (`None`) a soft claim on one element.
    ClaimUpdate {
        element_id: ElementId,
        claim: Option<SoftClaim>,
    },
    /// Host clock/sequence heartbeat.
    ClockSync { clock_ms: u64, next_seq: u64 },
    /// Periodic divergence check report.
    HashCheck { result: HashCheckResult },
}

/// Why a co-play operation was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoplayError {
    /// The session already holds `max_players` players.
    SessionFull { max_players: usize },
    /// The named player is not registered.
    UnknownPlayer(String),
    /// The player's role (or edit permission) forbids the operation.
    PermissionDenied,
    /// The peer speaks a different protocol version.
    VersionMismatch { expected: u32, got: u32 },
    /// The snapshot names a site this build does not know.
    UnknownSite(String),
    /// The snapshot names a style pack this build does not know.
    UnknownStyle(String),
    /// The peer's style pack contents differ from ours.
    StylePackHashMismatch,
    /// The snapshot document failed validation.
    InvalidDocument(String),
    /// A command-layer error, for example an invalid author id or unknown
    /// element.
    Command(CommandError),
}

impl fmt::Display for CoplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionFull { max_players } => {
                write!(f, "the session is full ({max_players} players)")
            }
            Self::UnknownPlayer(id) => write!(f, "unknown player {id}"),
            Self::PermissionDenied => write!(f, "player role does not permit this action"),
            Self::VersionMismatch { expected, got } => write!(
                f,
                "co-play version mismatch: expected {expected}, got {got}"
            ),
            Self::UnknownSite(site_id) => write!(f, "unknown site {site_id}"),
            Self::UnknownStyle(style_id) => write!(f, "unknown style pack {style_id}"),
            Self::StylePackHashMismatch => {
                write!(f, "style pack contents do not match the host")
            }
            Self::InvalidDocument(reason) => write!(f, "invalid snapshot document: {reason}"),
            Self::Command(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CoplayError {}

impl From<CommandError> for CoplayError {
    fn from(error: CommandError) -> Self {
        Self::Command(error)
    }
}

/// Deterministic 16-hex FNV-1a 64-bit hash of a style pack's canonical JSON.
/// Clients compare this on join so two different packs with the same id can
/// never silently diverge (architecture section 15.1).
pub fn style_pack_hash(style: &StylePack) -> String {
    // Serialising a `StylePack` cannot fail: it is a flat struct of scalars,
    // string lists and `f64`s. Fall back to the id so this helper stays
    // infallible.
    let canonical = serde_json::to_string(style).unwrap_or_else(|_| style.id.clone());
    format!("{:016x}", crate::fnv1a_64(canonical.as_bytes()))
}

/// Classify a document/layout hash comparison.
///
/// A document mismatch takes priority: if the documents differ, comparing
/// generated layout tells us nothing useful.
pub fn compare_hashes(
    host_doc_hash: &str,
    host_layout_hash: &str,
    client_doc_hash: &str,
    client_layout_hash: &str,
) -> HashCheckResult {
    if host_doc_hash != client_doc_hash {
        return HashCheckResult::DocumentMismatch {
            host_doc_hash: host_doc_hash.to_string(),
            client_doc_hash: client_doc_hash.to_string(),
        };
    }
    if host_layout_hash != client_layout_hash {
        return HashCheckResult::GeneratorDivergence {
            doc_hash: host_doc_hash.to_string(),
            host_layout_hash: host_layout_hash.to_string(),
            client_layout_hash: client_layout_hash.to_string(),
        };
    }
    HashCheckResult::Match
}

/// Serde adapter representing a [`CastleDocument`] as element and opening
/// arrays. Ids are carried inside each value, so the representation is
/// lossless and canonical order is preserved.
mod document_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::document::{CastleDocument, Element, Opening};

    #[derive(Serialize, Deserialize)]
    struct DocumentWire {
        elements: Vec<Element>,
        openings: Vec<Opening>,
    }

    pub fn serialize<S: Serializer>(
        document: &CastleDocument,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        DocumentWire {
            elements: document.elements().cloned().collect(),
            openings: document.openings().cloned().collect(),
        }
        .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<CastleDocument, D::Error> {
        let wire = DocumentWire::deserialize(deserializer)?;
        let mut document = CastleDocument::new();
        for element in wire.elements {
            document.insert(element);
        }
        for opening in wire.openings {
            document.insert_opening(opening);
        }
        Ok(document)
    }
}

/// Hex document hash used on the wire.
pub(crate) fn document_hash_hex(document: &CastleDocument) -> String {
    format!("{:016x}", document.document_hash())
}

/// Hex layout hash used on the wire.
pub(crate) fn layout_hash_hex(layout_hash: u64) -> String {
    format!("{layout_hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;

    fn player(id: &str) -> AuthorId {
        AuthorId::player(id).unwrap()
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

    fn session_with_castle() -> Session {
        let mut session = Session::new_empty();
        session
            .apply_batch(
                player("alex"),
                vec![
                    wall("North curtain", [-8_000, 0], [18_000, 0]),
                    wall("South curtain", [0, 10_000], [10_000, 10_000]),
                ],
            )
            .unwrap();
        session
    }

    #[test]
    fn default_player_limit_is_two_and_can_be_raised_to_four() {
        let mut session = Session::new_empty();
        assert_eq!(session.max_players(), DEFAULT_MAX_PLAYERS);

        session
            .register_player("alex", "Alex", "#e8b04b", PlayerRole::Host)
            .unwrap();
        session
            .register_player("bryn", "Bryn", "#4b9be8", PlayerRole::Builder)
            .unwrap();
        assert_eq!(
            session.register_player("cara", "Cara", "#8be84b", PlayerRole::Viewer),
            Err(CoplayError::SessionFull { max_players: 2 })
        );

        session.set_max_players(99);
        assert_eq!(session.max_players(), MAX_PLAYERS_LIMIT);
        session
            .register_player("cara", "Cara", "#8be84b", PlayerRole::Viewer)
            .unwrap();
        session
            .register_player("dafydd", "Dafydd", "#e84b9b", PlayerRole::Builder)
            .unwrap();
        assert_eq!(session.players().count(), 4);
        assert_eq!(
            session.register_player("eira", "Eira", "#999999", PlayerRole::Builder),
            Err(CoplayError::SessionFull { max_players: 4 })
        );

        // Clamped from below too.
        session.set_max_players(0);
        assert_eq!(session.max_players(), 1);
    }

    #[test]
    fn viewers_cannot_edit_but_hosts_and_builders_can() {
        let mut session = Session::new_empty();
        session.set_max_players(3);
        session
            .register_player("alex", "Alex", "#e8b04b", PlayerRole::Host)
            .unwrap();
        session
            .register_player("bryn", "Bryn", "#4b9be8", PlayerRole::Builder)
            .unwrap();
        session
            .register_player("cara", "Cara", "#8be84b", PlayerRole::Viewer)
            .unwrap();

        assert!(session.ensure_player_can_edit(&player("alex")).is_ok());
        assert!(session.ensure_player_can_edit(&player("bryn")).is_ok());
        assert_eq!(
            session.ensure_player_can_edit(&player("cara")),
            Err(CoplayError::PermissionDenied)
        );
        // Agents are governed by their own permission gate, not player roles.
        assert!(session
            .ensure_player_can_edit(&AuthorId::agent("mason").unwrap())
            .is_ok());
    }

    #[test]
    fn snapshot_round_trip_matches_document_and_layout_hashes() {
        let session = session_with_castle();
        let snapshot = session.build_coplay_snapshot();
        assert_eq!(snapshot.handshake.generator_version, GENERATOR_VERSION);
        assert_eq!(snapshot.handshake.site_id, session.site_id());
        assert_eq!(snapshot.document, *session.document());

        let restored = Session::from_coplay_snapshot(snapshot.clone()).unwrap();
        assert_eq!(restored.document(), session.document());
        assert_eq!(restored.next_element_id(), session.next_element_id());
        assert_eq!(restored.next_seq(), session.next_seq());
        assert_eq!(
            restored.verify_coplay_hashes(&snapshot.document_hash, &snapshot.layout_hash),
            HashCheckResult::Match
        );

        // The restored session keeps building from the same allocator.
        let mut restored = restored;
        let outcome = restored
            .apply_batch(
                player("alex"),
                vec![wall("East", [20_000, 0], [20_000, 10_000])],
            )
            .unwrap();
        assert_eq!(outcome.seq, session.next_seq());
        assert_eq!(outcome.touched_elements, vec![3]);
    }

    #[test]
    fn snapshot_preserves_players_claims_and_max_players() {
        let mut session = session_with_castle();
        session.set_max_players(3);
        session
            .register_player("alex", "Alex", "#e8b04b", PlayerRole::Host)
            .unwrap();
        session
            .register_player("bryn", "Bryn", "#4b9be8", PlayerRole::Builder)
            .unwrap();
        session.update_player_presence(
            "bryn",
            [1_000, 2_000, 3_000],
            [4_000, 5_000, 6_000],
            "building",
        );
        session
            .claim_element(1, player("bryn"), "laying the north course")
            .unwrap();

        let snapshot = session.build_coplay_snapshot();
        assert_eq!(snapshot.players.len(), 2);
        assert_eq!(snapshot.claims.len(), 1);
        assert_eq!(snapshot.handshake.max_players, 3);

        let restored = Session::from_coplay_snapshot(snapshot).unwrap();
        assert_eq!(restored.max_players(), 3);
        assert_eq!(restored.players().count(), 2);
        assert_eq!(restored.player("bryn").unwrap().status, "building");
        assert_eq!(
            restored.player("bryn").unwrap().camera_pos_mm,
            [1_000, 2_000, 3_000]
        );
        assert_eq!(restored.claim_for(1).unwrap().owner, player("bryn"));
    }

    #[test]
    fn from_snapshot_rejects_wrong_version_site_and_style_hash() {
        let session = session_with_castle();
        let snapshot = session.build_coplay_snapshot();

        let mut bad_version = snapshot.clone();
        bad_version.handshake.generator_version = GENERATOR_VERSION + 1;
        assert_eq!(
            Session::from_coplay_snapshot(bad_version).unwrap_err(),
            CoplayError::VersionMismatch {
                expected: GENERATOR_VERSION,
                got: GENERATOR_VERSION + 1,
            }
        );

        let mut bad_site = snapshot.clone();
        bad_site.handshake.site_id = "atlantis".to_string();
        assert_eq!(
            Session::from_coplay_snapshot(bad_site).unwrap_err(),
            CoplayError::UnknownSite("atlantis".to_string())
        );

        let mut bad_style = snapshot.clone();
        bad_style.handshake.style_id = "baroque".to_string();
        assert_eq!(
            Session::from_coplay_snapshot(bad_style).unwrap_err(),
            CoplayError::UnknownStyle("baroque".to_string())
        );

        let mut bad_hash = snapshot;
        bad_hash.handshake.style_pack_hash = "0000000000000000".to_string();
        assert_eq!(
            Session::from_coplay_snapshot(bad_hash).unwrap_err(),
            CoplayError::StylePackHashMismatch
        );
    }

    #[test]
    fn from_snapshot_rejects_an_invalid_document() {
        let session = session_with_castle();
        let mut snapshot = session.build_coplay_snapshot();
        // Corrupt an opening so it references a wall that does not exist. The
        // snapshot is untrusted input, exactly like a save file.
        snapshot.document = CastleDocument::new();
        snapshot
            .document
            .add_opening(crate::document::Opening::arrow_loop(
                5, 99, "orphan", 500, 1_500,
            ));
        snapshot.next_element_id = 6;
        assert!(matches!(
            Session::from_coplay_snapshot(snapshot),
            Err(CoplayError::InvalidDocument(_))
        ));
        // The live session is unaffected.
        assert!(!session.document().is_empty());
    }

    #[test]
    fn snapshot_restore_drops_invalid_claims_and_enforces_player_cap() {
        let mut session = session_with_castle();
        session.set_max_players(2);
        session
            .register_player("alex", "Alex", "#e8b04b", PlayerRole::Host)
            .unwrap();
        session
            .register_player("bryn", "Bryn", "#4b9be8", PlayerRole::Builder)
            .unwrap();
        session
            .claim_element(1, player("bryn"), "north wall")
            .unwrap();

        let mut snapshot = session.build_coplay_snapshot();
        // A hostile snapshot can append players beyond the handshake cap.
        snapshot.players.push(PlayerParticipant {
            id: "cara".to_string(),
            name: "Cara".to_string(),
            color_hex: "#8be84b".to_string(),
            role: PlayerRole::Builder,
            camera_pos_mm: [0, 0, 0],
            camera_target_mm: [0, 0, 0],
            status: String::new(),
        });
        // ...and claims that reference missing elements or carry unsafe notes.
        snapshot.claims.push(SoftClaim {
            element_id: 999,
            owner: player("bryn"),
            note: "ghost wall".to_string(),
        });
        snapshot.claims.push(SoftClaim {
            element_id: 2,
            owner: player("bryn"),
            note: "bad\nnote".to_string(),
        });

        let restored = Session::from_coplay_snapshot(snapshot).unwrap();
        assert_eq!(restored.players().count(), 2, "player cap must be enforced");
        assert!(restored.player("cara").is_none());
        assert_eq!(
            restored.claims().count(),
            1,
            "invalid claims must be dropped"
        );
        assert_eq!(restored.claim_for(1).unwrap().owner, player("bryn"));
        assert!(restored.claim_for(999).is_none());
        assert!(restored.claim_for(2).is_none());
    }

    #[test]
    fn snapshot_restore_never_lowers_the_element_id_allocator() {
        let session = session_with_castle();
        let mut snapshot = session.build_coplay_snapshot();
        // A bad snapshot claims the next id is 1 even though ids 1 and 2 exist.
        snapshot.next_element_id = 1;

        let mut restored = Session::from_coplay_snapshot(snapshot).unwrap();
        assert_eq!(
            restored.next_element_id(),
            3,
            "next_element_id must stay above the highest existing element id"
        );

        let outcome = restored
            .apply_batch(
                player("alex"),
                vec![wall("East", [20_000, 0], [20_000, 10_000])],
            )
            .unwrap();
        assert_eq!(outcome.touched_elements, vec![3]);
    }

    #[test]
    fn snapshot_restore_rejects_a_tampered_document_hash() {
        let session = session_with_castle();
        let mut snapshot = session.build_coplay_snapshot();
        snapshot.document_hash = "0000000000000000".to_string();
        assert_eq!(
            Session::from_coplay_snapshot(snapshot).unwrap_err(),
            CoplayError::InvalidDocument("snapshot document hash mismatch".into())
        );
    }

    #[test]
    fn compare_hashes_classifies_matches_and_divergence() {
        assert_eq!(
            compare_hashes("aa", "bb", "aa", "bb"),
            HashCheckResult::Match
        );
        assert_eq!(
            compare_hashes("aa", "bb", "cc", "bb"),
            HashCheckResult::DocumentMismatch {
                host_doc_hash: "aa".to_string(),
                client_doc_hash: "cc".to_string(),
            }
        );
        assert_eq!(
            compare_hashes("aa", "bb", "aa", "cc"),
            HashCheckResult::GeneratorDivergence {
                doc_hash: "aa".to_string(),
                host_layout_hash: "bb".to_string(),
                client_layout_hash: "cc".to_string(),
            }
        );
    }

    #[test]
    fn verify_coplay_hashes_reports_against_the_live_session() {
        let session = session_with_castle();
        let snapshot = session.build_coplay_snapshot();
        assert_eq!(
            session.verify_coplay_hashes(&snapshot.document_hash, &snapshot.layout_hash),
            HashCheckResult::Match
        );
        assert!(matches!(
            session.verify_coplay_hashes(&snapshot.document_hash, "0000000000000000"),
            HashCheckResult::GeneratorDivergence { .. }
        ));
        assert!(matches!(
            session.verify_coplay_hashes("0000000000000000", &snapshot.layout_hash),
            HashCheckResult::DocumentMismatch { .. }
        ));
    }

    #[test]
    fn style_pack_hash_is_stable_and_distinguishes_packs() {
        let edwardian = StylePack::edwardian_welsh();
        let baltic = StylePack::baltic_brick();
        let hash = style_pack_hash(&edwardian);
        assert_eq!(hash.len(), 16);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hash, style_pack_hash(&StylePack::edwardian_welsh()));
        assert_ne!(hash, style_pack_hash(&baltic));

        // A changed rule changes the hash even though the id is the same.
        let mut modified = edwardian;
        modified.course_height_mm += 1;
        assert_ne!(hash, style_pack_hash(&modified));
    }

    #[test]
    fn soft_claims_create_replace_and_release() {
        let mut session = session_with_castle();
        session
            .register_player("alex", "Alex", "#e8b04b", PlayerRole::Host)
            .unwrap();
        session
            .register_player("bryn", "Bryn", "#4b9be8", PlayerRole::Builder)
            .unwrap();

        let claim = session
            .claim_element(1, player("bryn"), "north wall foundations")
            .unwrap();
        assert_eq!(claim.element_id, 1);
        assert_eq!(claim.owner, player("bryn"));
        assert_eq!(claim.note, "north wall foundations");
        assert_eq!(session.claims().count(), 1);

        // Alex (host) may take the claim over; replacement is in place.
        session
            .claim_element(1, player("alex"), "checking the bond")
            .unwrap();
        assert_eq!(session.claims().count(), 1, "claims are keyed by element");
        assert_eq!(session.claim_for(1).unwrap().owner, player("alex"));

        // A non-owner builder cannot release someone else's claim...
        assert!(!session.release_claim(1, &player("bryn")));
        assert!(session.claim_for(1).is_some());
        // ...but the owner and the host both can.
        assert!(session.release_claim(1, &player("alex")));
        assert!(session.claim_for(1).is_none());

        // Claiming a missing element is refused.
        assert_eq!(
            session.claim_element(999, player("bryn"), "ghost"),
            Err(CoplayError::Command(CommandError::UnknownElementId(999)))
        );
    }

    #[test]
    fn claims_are_cleaned_up_when_a_player_disconnects() {
        let mut session = session_with_castle();
        session
            .register_player("alex", "Alex", "#e8b04b", PlayerRole::Host)
            .unwrap();
        session
            .register_player("bryn", "Bryn", "#4b9be8", PlayerRole::Builder)
            .unwrap();
        session
            .claim_element(1, player("bryn"), "north wall")
            .unwrap();
        session
            .claim_element(2, player("alex"), "south wall")
            .unwrap();

        assert!(session.remove_player("bryn"));
        assert_eq!(session.claim_for(1), None);
        assert_eq!(session.claim_for(2).unwrap().owner, player("alex"));
        assert!(!session.remove_player("bryn"), "second removal is a no-op");
    }

    #[test]
    fn claim_notes_are_sanitised_and_status_truncated_safely() {
        let mut session = session_with_castle();
        session
            .register_player("bryn", "  Bryn  ", "#4b9be8", PlayerRole::Builder)
            .unwrap();
        assert_eq!(session.player("bryn").unwrap().name, "Bryn");
        // A malformed colour falls back to neutral grey.
        session
            .register_player("cara", "Cara", "not-a-colour", PlayerRole::Viewer)
            .unwrap();
        assert_eq!(session.player("cara").unwrap().color_hex, "#888888");

        let claim = session
            .claim_element(1, player("bryn"), "  north wall  ")
            .unwrap();
        assert_eq!(claim.note, "north wall");

        // Control characters cannot enter presence text; the update itself
        // still succeeds (the player exists) but the unsafe text is dropped.
        assert!(session.update_player_presence("bryn", [0, 0, 0], [1, 1, 1], "bad\nstatus"));
        assert_eq!(session.player("bryn").unwrap().status, "");

        assert!(session.update_player_presence("bryn", [0, 0, 0], [1, 1, 1], "  surveying  "));
        assert_eq!(session.player("bryn").unwrap().status, "surveying");
        assert!(!session.update_player_presence("ghost", [0, 0, 0], [0, 0, 0], "x"));

        // Unknown players cannot change role.
        assert_eq!(
            session.set_player_role("ghost", PlayerRole::Builder),
            Err(CoplayError::UnknownPlayer("ghost".to_string()))
        );
        assert!(session.set_player_role("cara", PlayerRole::Builder).is_ok());
        assert!(session.ensure_player_can_edit(&player("cara")).is_ok());
    }

    #[test]
    fn invalid_player_ids_are_refused() {
        let mut session = Session::new_empty();
        assert!(matches!(
            session.register_player("   ", "Nobody", "#ffffff", PlayerRole::Builder),
            Err(CoplayError::Command(CommandError::AuthorIdEmpty))
        ));
        assert!(matches!(
            session.register_player("bad\nname", "Nobody", "#ffffff", PlayerRole::Builder),
            Err(CoplayError::Command(CommandError::AuthorIdHasControlChars))
        ));
        assert!(session.players().next().is_none());
    }

    #[test]
    fn coplay_messages_round_trip_through_json() {
        let session = session_with_castle();
        let snapshot = session.build_coplay_snapshot();
        let handshake = snapshot.handshake.clone();
        let participant = PlayerParticipant {
            id: "bryn".to_string(),
            name: "Bryn".to_string(),
            color_hex: "#4b9be8".to_string(),
            role: PlayerRole::Builder,
            camera_pos_mm: [1_000, 2_000, 3_000],
            camera_target_mm: [4_000, 5_000, 6_000],
            status: "surveying".to_string(),
        };
        let claim = SoftClaim {
            element_id: 1,
            owner: player("bryn"),
            note: "north wall".to_string(),
        };

        let messages = vec![
            CoplayMessage::JoinRequest {
                handshake: handshake.clone(),
                player: participant.clone(),
            },
            CoplayMessage::JoinAccepted {
                handshake,
                snapshot,
            },
            CoplayMessage::JoinRejected {
                reason: "session full".to_string(),
            },
            CoplayMessage::SubmitBatch {
                author: player("bryn"),
                ops: vec![wall("East", [20_000, 0], [20_000, 10_000])],
            },
            CoplayMessage::SubmitUndo {
                author: player("bryn"),
            },
            CoplayMessage::BroadcastBatch {
                seq: 2,
                command_id: "cmd-2".to_string(),
                author: player("bryn"),
                ops: vec![],
                document_hash: "0000000000000000".to_string(),
            },
            CoplayMessage::BroadcastUndo {
                seq: 3,
                command_id: "cmd-3".to_string(),
                author: player("bryn"),
                target_seq: 2,
                document_hash: "0000000000000000".to_string(),
            },
            CoplayMessage::PresenceUpdate {
                player_id: "bryn".to_string(),
                camera_pos_mm: [1, 2, 3],
                camera_target_mm: [4, 5, 6],
                status: "surveying".to_string(),
            },
            CoplayMessage::ClaimUpdate {
                element_id: 1,
                claim: Some(claim),
            },
            CoplayMessage::ClaimUpdate {
                element_id: 1,
                claim: None,
            },
            CoplayMessage::ClockSync {
                clock_ms: 1_700_000_000_000,
                next_seq: 4,
            },
            CoplayMessage::HashCheck {
                result: HashCheckResult::DocumentMismatch {
                    host_doc_hash: "aa".to_string(),
                    client_doc_hash: "bb".to_string(),
                },
            },
        ];

        for (index, message) in messages.into_iter().enumerate() {
            let json = serde_json::to_string(&message).unwrap();
            let back: CoplayMessage = serde_json::from_str(&json)
                .unwrap_or_else(|error| panic!("message {index}: {error} in {json}"));
            assert_eq!(back, message);
        }
    }

    #[test]
    fn coplay_message_json_uses_snake_case_type_tags() {
        let message = CoplayMessage::ClockSync {
            clock_ms: 1,
            next_seq: 2,
        };
        let json = serde_json::to_string(&message).unwrap();
        assert!(json.contains("\"type\":\"clock_sync\""), "{json}");

        let result = HashCheckResult::GeneratorDivergence {
            doc_hash: "aa".to_string(),
            host_layout_hash: "bb".to_string(),
            client_layout_hash: "cc".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"kind\":\"generator_divergence\""), "{json}");

        assert_eq!(
            serde_json::from_str::<PlayerRole>("\"builder\"").unwrap(),
            PlayerRole::Builder
        );
        assert_eq!(
            serde_json::from_str::<PlayerRole>("\"viewer\"").unwrap(),
            PlayerRole::Viewer
        );
    }
}
