//! Host-authoritative in-process session.
//!
//! The session owns the authoritative [`CastleDocument`], the append-only
//! [`LoggedCommand`] history, the monotonic element id allocator and the
//! deterministic clock used for UUIDv7 command ids. Single player, MCP agents
//! and (later) networked guests all speak to the same command bus, so the host
//! validates every edit exactly once (architecture sections 4.2, 13 M1 and 15).
//!
//! Undo never truncates history: it appends a [`CommandPayload::CompensateUndo`]
//! that applies the exact inverse snapshot recorded with its target command.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::command::{
    execute_batch, uuid_v7, validate_element_budget, AuthorId, CommandError, CommandOp,
    CommandPayload, InverseRecord, LoggedCommand,
};
use crate::document::{CastleDocument, Element, ElementId, Opening};
use crate::junction::junction_partners;
use crate::mesh::{build_scene, SceneData};
use crate::terrain::SiteTerrain;

/// Default site id used by the M0a fixtures.
pub const DEFAULT_SITE_ID: &str = "m0a_promontory";

/// Deterministic start of the session clock (2023-11-14T22:13:20Z). UUIDv7 ids
/// only need to be unique and monotonic, not wall-clock accurate.
pub const DEFAULT_CLOCK_MS: u64 = 1_700_000_000_000;

/// Result of a successful [`Session::apply_batch`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyOutcome {
    pub seq: u64,
    pub command_id: String,
    pub created_ids: BTreeMap<String, ElementId>,
    pub touched_elements: Vec<ElementId>,
    pub document_hash: u64,
    pub summary: String,
}

/// Result of a successful [`Session::undo_last`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoOutcome {
    /// Sequence number of the newly appended compensating command.
    pub undo_seq: u64,
    /// Sequence number of the batch that was compensated.
    pub target_seq: u64,
    pub command_id: String,
    pub document_hash: u64,
    pub summary: String,
}

/// Why an undo could not be applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoError {
    /// The author has no undoable (still-live) batch in the history.
    NoUndoableCommand { author: AuthorId },
    /// A later command by someone else touched an element this undo would
    /// change, so undoing would silently destroy their work (sections 4.2 and
    /// 15.3).
    BlockedByLaterCommand {
        target_seq: u64,
        blocking_seq: u64,
        blocking_author: AuthorId,
        element_id: ElementId,
    },
    /// Replaying the inverse failed (only possible with a corrupted log).
    Command(CommandError),
}

impl fmt::Display for UndoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoUndoableCommand { author } => {
                write!(f, "{author} has no command left to undo")
            }
            Self::BlockedByLaterCommand {
                target_seq,
                blocking_seq,
                blocking_author,
                element_id,
            } => write!(
                f,
                "cannot undo command {target_seq}: command {blocking_seq} by \
                 {blocking_author} changed element {element_id} afterwards"
            ),
            Self::Command(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for UndoError {}

impl From<CommandError> for UndoError {
    fn from(error: CommandError) -> Self {
        Self::Command(error)
    }
}

/// Authoritative in-process castle session.
#[derive(Clone, Debug)]
pub struct Session {
    pub(crate) site_id: String,
    pub(crate) terrain: SiteTerrain,
    /// Document before any command in `command_log` was applied. Replaying the
    /// log onto this document reproduces `document` exactly.
    pub(crate) initial_document: CastleDocument,
    pub(crate) document: CastleDocument,
    pub(crate) command_log: Vec<LoggedCommand>,
    /// Sequence numbers of batch commands already compensated by a later undo.
    pub(crate) undone_seqs: BTreeSet<u64>,
    /// Monotonic id allocator; ids are never reused, even after deletion.
    pub(crate) next_element_id: ElementId,
    pub(crate) next_seq: u64,
    /// Deterministic monotonic millisecond clock for UUIDv7.
    pub(crate) clock_ms: u64,
}

impl Session {
    /// A session with an empty document on the default M0a site.
    pub fn new_empty() -> Self {
        Self::from_document(
            DEFAULT_SITE_ID,
            SiteTerrain::m0a_promontory(),
            CastleDocument::new(),
        )
    }

    /// A session seeded with an existing document. The id allocator starts
    /// above the highest existing element or opening id.
    pub fn from_document(
        site_id: impl Into<String>,
        terrain: SiteTerrain,
        document: CastleDocument,
    ) -> Self {
        let next_element_id = max_existing_id(&document) + 1;
        Self {
            site_id: site_id.into(),
            terrain,
            initial_document: document.clone(),
            document,
            command_log: Vec::new(),
            undone_seqs: BTreeSet::new(),
            next_element_id,
            next_seq: 1,
            clock_ms: DEFAULT_CLOCK_MS,
        }
    }

    pub fn site_id(&self) -> &str {
        &self.site_id
    }

    pub fn document(&self) -> &CastleDocument {
        &self.document
    }

    pub fn terrain(&self) -> &SiteTerrain {
        &self.terrain
    }

    pub fn history(&self) -> &[LoggedCommand] {
        &self.command_log
    }

    pub fn next_element_id(&self) -> ElementId {
        self.next_element_id
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub fn is_undone(&self, seq: u64) -> bool {
        self.undone_seqs.contains(&seq)
    }

    /// Generate the scene for the current document and terrain.
    pub fn build_scene(&self) -> SceneData {
        build_scene(&self.document, &self.terrain)
    }

    /// Apply a batch atomically. All operations are validated and applied to a
    /// clone; the live document, id allocator and log are only advanced when
    /// the whole batch succeeds.
    pub fn apply_batch(
        &mut self,
        author: AuthorId,
        ops: Vec<CommandOp>,
    ) -> Result<ApplyOutcome, CommandError> {
        // An empty batch is a successful no-op. Logging it would waste the
        // author's next undo on a command that changed nothing (M8/L4).
        if ops.is_empty() {
            return Ok(ApplyOutcome {
                seq: self.next_seq,
                command_id: uuid_v7(self.clock_ms, self.next_seq),
                created_ids: BTreeMap::new(),
                touched_elements: Vec::new(),
                document_hash: self.document.document_hash(),
                summary: "no changes".to_string(),
            });
        }

        let application = execute_batch(&self.document, self.next_element_id, &ops)?;

        let seq = self.next_seq;
        let command_id = uuid_v7(self.clock_ms, seq);
        let summary = application.summary.clone();
        let touched_elements = application.touched_elements.clone();
        let created_ids = application.created_ids.clone();

        // Record every element that shared a junction with a touched element
        // either before or after the edit. Undo uses this so removing one half
        // of a junction cannot silently orphan the other (H4).
        let mut junctions: BTreeSet<ElementId> = BTreeSet::new();
        for id in &touched_elements {
            junctions.extend(junction_partners(&self.document, &self.terrain, *id));
            junctions.extend(junction_partners(&application.document, &self.terrain, *id));
        }

        self.document = application.document;
        self.next_element_id = application.next_element_id;
        self.command_log.push(LoggedCommand {
            seq,
            command_id: command_id.clone(),
            author,
            payload: CommandPayload::Batch { ops },
            touched_elements: touched_elements.clone(),
            junction_elements: junctions.into_iter().collect(),
            inverse: application.inverse,
            summary: summary.clone(),
        });
        self.next_seq += 1;
        self.clock_ms += 1;

        Ok(ApplyOutcome {
            seq,
            command_id,
            created_ids,
            touched_elements,
            document_hash: self.document.document_hash(),
            summary,
        })
    }

    /// Compensate the author's latest still-live batch, appending a new undo
    /// command. Refused when a later command by anyone touched the same
    /// elements (architecture sections 4.2 and 15.3).
    pub fn undo_last(&mut self, author: &AuthorId) -> Result<UndoOutcome, UndoError> {
        let target_index = self
            .command_log
            .iter()
            .rposition(|command| {
                command.author == *author
                    && matches!(command.payload, CommandPayload::Batch { .. })
                    && !self.undone_seqs.contains(&command.seq)
            })
            .ok_or_else(|| UndoError::NoUndoableCommand {
                author: author.clone(),
            })?;
        let target = self.command_log[target_index].clone();

        // Refuse if any later, still-live batch touched an element this undo
        // would change. Openings record their parent wall in `touched_elements`
        // so an opening added to a wall also blocks undoing that wall. The
        // scope also includes every junction partner the target had when it was
        // first applied and any it has in the current document, because editing
        // either half of a junction changes the other's masonry (H4).
        let mut target_scope: BTreeSet<ElementId> =
            target.touched_elements.iter().copied().collect();
        target_scope.extend(target.junction_elements.iter().copied());
        for id in &target.touched_elements {
            target_scope.extend(junction_partners(&self.document, &self.terrain, *id));
        }

        if let Some(blocking) = self.command_log.iter().find(|command| {
            command.seq > target.seq
                && !self.undone_seqs.contains(&command.seq)
                && matches!(command.payload, CommandPayload::Batch { .. })
                && command
                    .touched_elements
                    .iter()
                    .any(|id| target_scope.contains(id))
        }) {
            let element_id = blocking
                .touched_elements
                .iter()
                .copied()
                .filter(|id| target_scope.contains(id))
                .min()
                .expect("intersection is non-empty");
            return Err(UndoError::BlockedByLaterCommand {
                target_seq: target.seq,
                blocking_seq: blocking.seq,
                blocking_author: blocking.author.clone(),
                element_id,
            });
        }

        // Apply the exact inverse snapshot to a clone first: restoring deleted
        // elements must not push the document over its element budget (M8).
        let mut restored = self.document.clone();
        target.inverse.apply(&mut restored);
        validate_element_budget(restored.len(), restored.opening_count())
            .map_err(UndoError::Command)?;
        self.document = restored;
        self.undone_seqs.insert(target.seq);

        let seq = self.next_seq;
        let command_id = uuid_v7(self.clock_ms, seq);
        let summary = format!("Undo command #{}", target.seq);
        let touched_elements = target.touched_elements.clone();
        self.command_log.push(LoggedCommand {
            seq,
            command_id: command_id.clone(),
            author: author.clone(),
            payload: CommandPayload::CompensateUndo {
                target_seq: target.seq,
            },
            touched_elements,
            junction_elements: target.junction_elements.clone(),
            inverse: InverseRecord::default(),
            summary: summary.clone(),
        });
        self.next_seq += 1;
        self.clock_ms += 1;

        Ok(UndoOutcome {
            undo_seq: seq,
            target_seq: target.seq,
            command_id,
            document_hash: self.document.document_hash(),
            summary,
        })
    }

    /// Replay the command log on a fresh session seeded with
    /// `initial_document`, proving deterministic equivalence (section 15.5).
    pub fn replay(&self) -> Result<CastleDocument, CommandError> {
        Ok(replay_commands(
            &self.site_id,
            &self.terrain,
            &self.initial_document,
            &self.command_log,
        )?
        .document)
    }
}

/// Replay a command log from an initial document. Shared by [`Session::replay`]
/// and save/load verification.
pub(crate) fn replay_commands(
    site_id: &str,
    terrain: &SiteTerrain,
    initial_document: &CastleDocument,
    command_log: &[LoggedCommand],
) -> Result<Session, CommandError> {
    let mut session = Session::from_document(
        site_id.to_string(),
        terrain.clone(),
        initial_document.clone(),
    );

    for command in command_log {
        match &command.payload {
            CommandPayload::Batch { ops } => {
                let application = execute_batch(&session.document, session.next_element_id, ops)?;
                session.document = application.document;
                session.next_element_id = application.next_element_id;
                session.next_seq = session.next_seq.max(command.seq + 1);
                session.clock_ms = session.clock_ms.max(DEFAULT_CLOCK_MS + command.seq + 1);
            }
            CommandPayload::CompensateUndo { target_seq } => {
                let target = command_log
                    .iter()
                    .find(|entry| entry.seq == *target_seq)
                    .ok_or(CommandError::UnknownTargetSeq(*target_seq))?;
                target.inverse.apply(&mut session.document);
                session.undone_seqs.insert(*target_seq);
                session.next_seq = session.next_seq.max(command.seq + 1);
                session.clock_ms = session.clock_ms.max(DEFAULT_CLOCK_MS + command.seq + 1);
            }
        }
    }

    Ok(session)
}

/// Highest element or opening id currently present in a document.
pub(crate) fn max_existing_id(document: &CastleDocument) -> ElementId {
    let max_element = document.elements().map(Element::id).max().unwrap_or(0);
    let max_opening = document
        .openings()
        .map(|opening: &Opening| opening.id)
        .max()
        .unwrap_or(0);
    max_element.max(max_opening)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{AuthorKind, MAX_BATCH_OPS};
    use crate::document::{BatteredWall, OpeningKind, RoundTower};

    fn player() -> AuthorId {
        AuthorId::player("alex").unwrap()
    }

    fn agent() -> AuthorId {
        AuthorId::agent("mason").unwrap()
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

    fn tower(name: &str, centre: [i64; 2]) -> CommandOp {
        CommandOp::CreateTower {
            ref_id: None,
            name: name.into(),
            centre_mm: centre,
            radius_mm: 4_000,
            height_mm: 14_000,
            batter_mm: None,
        }
    }

    fn loop_op(wall_id: ElementId, t: i64) -> CommandOp {
        CommandOp::AddArrowLoop {
            wall_id: Some(wall_id),
            wall_ref: None,
            t_permille: t,
            sill_height_mm: 1_800,
            height_mm: None,
            outer_slit_mm: None,
            inner_splay_width_mm: None,
        }
    }

    #[test]
    fn new_empty_has_no_elements_and_allocates_from_one() {
        let session = Session::new_empty();
        assert_eq!(session.document().len(), 0);
        assert_eq!(session.next_element_id(), 1);
        assert_eq!(session.next_seq(), 1);
        assert!(session.history().is_empty());
    }

    #[test]
    fn from_document_starts_above_existing_ids() {
        let mut doc = CastleDocument::new();
        doc.add_wall(BatteredWall::new(7, "w", [0, 0], [10_000, 0], 9_000, 2_400));
        doc.add_opening(crate::document::Opening::arrow_loop(
            12, 7, "loop", 500, 1_500,
        ));
        let session = Session::from_document(DEFAULT_SITE_ID, SiteTerrain::m0a_promontory(), doc);
        assert_eq!(session.next_element_id(), 13);
        assert_eq!(session.initial_document, session.document);
    }

    #[test]
    fn apply_then_undo_restores_hashes_and_grows_history() {
        let mut session = Session::new_empty();
        let before_hash = session.document().document_hash();
        let before_layout = session.build_scene().layout_hash;

        let outcome = session
            .apply_batch(
                player(),
                vec![
                    wall("North curtain", [-8_000, 0], [18_000, 0]),
                    CommandOp::CreateWallRun {
                        ref_id: Some("north".into()),
                        name: "South curtain".into(),
                        start_mm: [0, 10_000],
                        end_mm: [10_000, 10_000],
                        height_mm: 8_000,
                        thickness_mm: 2_000,
                        batter_mm: Some(600),
                    },
                    CommandOp::AddArrowLoop {
                        wall_id: None,
                        wall_ref: Some("north".into()),
                        t_permille: 300,
                        sill_height_mm: 1_800,
                        height_mm: None,
                        outer_slit_mm: None,
                        inner_splay_width_mm: None,
                    },
                    tower("Drum tower", [20_000, 0]),
                ],
            )
            .unwrap();

        assert_eq!(outcome.seq, 1);
        assert_eq!(outcome.created_ids.len(), 1);
        assert_eq!(session.history().len(), 1);
        assert!(session.document().document_hash() != before_hash);
        assert_eq!(session.document().opening_count(), 1);

        let undo = session.undo_last(&player()).unwrap();
        assert_eq!(undo.target_seq, 1);
        assert_eq!(undo.undo_seq, 2);
        assert_eq!(session.document().document_hash(), before_hash);
        assert_eq!(session.build_scene().layout_hash, before_layout);
        assert_eq!(session.history().len(), 2, "undo appends, never truncates");
        assert!(matches!(
            session.history()[1].payload,
            CommandPayload::CompensateUndo { target_seq: 1 }
        ));
        assert!(session.is_undone(1));
        assert_eq!(session.document().len(), 0);
        assert_eq!(session.document().opening_count(), 0);
    }

    #[test]
    fn multi_author_independent_undo_is_allowed() {
        let mut session = Session::new_empty();
        session
            .apply_batch(player(), vec![wall("A wall", [0, 0], [10_000, 0])])
            .unwrap();
        session
            .apply_batch(agent(), vec![wall("B wall", [0, 20_000], [10_000, 20_000])])
            .unwrap();

        let a_wall = 1;
        let undo = session.undo_last(&player()).unwrap();
        assert_eq!(undo.target_seq, 1);
        assert_eq!(session.document().len(), 1);
        assert!(session.document().get(a_wall).is_none());
        let remaining: Vec<_> = session.document().elements().map(Element::id).collect();
        assert_eq!(remaining, vec![2], "the agent's wall survives");
    }

    #[test]
    fn multi_author_conflict_blocks_undo_until_the_dependent_work_is_undone() {
        let mut session = Session::new_empty();
        session
            .apply_batch(player(), vec![wall("A wall", [0, 0], [10_000, 0])])
            .unwrap();
        session.apply_batch(agent(), vec![loop_op(1, 500)]).unwrap();

        let error = session.undo_last(&player()).unwrap_err();
        match &error {
            UndoError::BlockedByLaterCommand {
                target_seq,
                blocking_seq,
                blocking_author,
                element_id,
            } => {
                assert_eq!(*target_seq, 1);
                assert_eq!(*blocking_seq, 2);
                assert_eq!(*blocking_author, agent());
                assert_eq!(*element_id, 1);
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert_eq!(session.document().len(), 1);
        assert_eq!(session.document().opening_count(), 1);

        // The agent undoes its arrow loop, after which the player's wall is
        // safe to remove again.
        session.undo_last(&agent()).unwrap();
        assert_eq!(session.document().opening_count(), 0);
        let undo = session.undo_last(&player()).unwrap();
        assert_eq!(undo.target_seq, 1);
        assert_eq!(session.document().len(), 0);
    }

    #[test]
    fn undo_without_a_command_is_refused() {
        let mut session = Session::new_empty();
        assert_eq!(
            session.undo_last(&player()),
            Err(UndoError::NoUndoableCommand { author: player() })
        );
    }

    #[test]
    fn a_failed_batch_is_atomic_and_not_logged() {
        let mut session = Session::new_empty();
        let before = session.document().document_hash();
        let error = session
            .apply_batch(
                player(),
                vec![
                    wall("North", [0, 0], [10_000, 0]),
                    CommandOp::DeleteElement { id: 999 },
                ],
            )
            .unwrap_err();
        assert_eq!(error, CommandError::UnknownElementId(999));
        assert_eq!(session.document().document_hash(), before);
        assert!(session.history().is_empty());
        assert_eq!(session.next_seq(), 1);
        assert_eq!(session.next_element_id(), 1);
    }

    #[test]
    fn oversized_batch_is_rejected() {
        let mut session = Session::new_empty();
        let ops = vec![CommandOp::DeleteElement { id: 1 }; MAX_BATCH_OPS + 1];
        assert!(matches!(
            session.apply_batch(player(), ops),
            Err(CommandError::BatchTooLarge { .. })
        ));
    }

    #[test]
    fn delete_and_undo_round_trips_elements_and_openings() {
        let mut session = Session::new_empty();
        session
            .apply_batch(
                player(),
                vec![wall("North", [0, 0], [10_000, 0]), loop_op(1, 300)],
            )
            .unwrap();
        let scene = session.build_scene();
        session
            .apply_batch(player(), vec![CommandOp::DeleteElement { id: 1 }])
            .unwrap();
        assert_eq!(session.document().len(), 0);
        assert_eq!(session.document().opening_count(), 0);

        session.undo_last(&player()).unwrap();
        assert_eq!(session.document().len(), 1);
        assert_eq!(session.document().opening_count(), 1);
        let restored = session.document().opening(2).unwrap();
        assert_eq!(restored.kind, OpeningKind::ArrowLoop);
        assert_eq!(restored.wall_id, 1);
        assert_eq!(session.build_scene().layout_hash, scene.layout_hash);
    }

    #[test]
    fn deleting_a_lone_opening_is_reversible() {
        let mut session = Session::new_empty();
        session
            .apply_batch(player(), vec![wall("North", [0, 0], [10_000, 0])])
            .unwrap();
        session.apply_batch(agent(), vec![loop_op(1, 600)]).unwrap();
        let before = session.document().document_hash();

        session
            .apply_batch(agent(), vec![CommandOp::DeleteElement { id: 2 }])
            .unwrap();
        assert_eq!(session.document().opening_count(), 0);
        session.undo_last(&agent()).unwrap();
        assert_eq!(session.document().document_hash(), before);
        assert_eq!(session.document().opening_count(), 1);
    }

    #[test]
    fn replay_is_deterministic_for_a_complex_multi_author_log() {
        let mut session = Session::from_document(
            DEFAULT_SITE_ID,
            SiteTerrain::m0a_promontory(),
            crate::fixtures::m0b_showcase_document(),
        );
        // Three authors building, deleting and undoing.
        session
            .apply_batch(
                player(),
                vec![
                    wall("West curtain", [-20_000, 12_000], [-20_000, -12_000]),
                    tower("West drum", [-22_000, 0]),
                ],
            )
            .unwrap();
        session
            .apply_batch(agent(), vec![loop_op(1, 200), loop_op(1, 800)])
            .unwrap();
        session
            .apply_batch(
                AuthorId::system(),
                vec![wall("South", [0, 20_000], [20_000, 20_000])],
            )
            .unwrap();
        // Undo the arrow loops before the return wall they join is deleted;
        // H4 now blocks undoing across a changed junction.
        session.undo_last(&agent()).unwrap(); // system wall stays
        session
            .apply_batch(player(), vec![CommandOp::DeleteElement { id: 2 }])
            .unwrap();
        session.undo_last(&player()).unwrap(); // restores deleted wall 2
        session
            .apply_batch(agent(), vec![tower("East drum", [30_000, 0])])
            .unwrap();

        let live_hash = session.document().document_hash();
        let live_layout = session.build_scene().layout_hash;

        let replayed = session.replay().unwrap();
        assert_eq!(replayed.document_hash(), live_hash);
        assert_eq!(
            build_scene(&replayed, session.terrain()).layout_hash,
            live_layout
        );
        assert_eq!(replayed, *session.document());
    }

    #[test]
    fn empty_batches_are_no_ops_that_do_not_consume_an_undo() {
        let mut session = Session::new_empty();
        let before = session.document().document_hash();
        let outcome = session.apply_batch(player(), Vec::new()).unwrap();
        assert_eq!(outcome.seq, 1);
        assert!(outcome.touched_elements.is_empty());
        assert_eq!(outcome.document_hash, before);
        assert!(
            session.history().is_empty(),
            "empty batch must not be logged"
        );
        assert_eq!(session.next_seq(), 1);
        assert_eq!(
            session.undo_last(&player()),
            Err(UndoError::NoUndoableCommand { author: player() })
        );
    }

    #[test]
    fn shared_junction_undo_is_refused_until_the_dependent_work_is_undone() {
        let mut session = Session::new_empty();
        // Player builds the tower; the agent's wall ends on its footprint.
        session
            .apply_batch(player(), vec![tower("Junction drum", [10_000, 0])])
            .unwrap();
        session
            .apply_batch(agent(), vec![wall("Agent wall", [10_000, 0], [30_000, 0])])
            .unwrap();

        let error = session.undo_last(&player()).unwrap_err();
        match &error {
            UndoError::BlockedByLaterCommand {
                target_seq,
                blocking_seq,
                blocking_author,
                ..
            } => {
                assert_eq!(*target_seq, 1);
                assert_eq!(*blocking_seq, 2);
                assert_eq!(*blocking_author, agent());
            }
            other => panic!("unexpected error: {other:?}"),
        }

        // Once the agent removes its wall the junction is gone and the tower
        // can be undone again.
        session.undo_last(&agent()).unwrap();
        let undo = session.undo_last(&player()).unwrap();
        assert_eq!(undo.target_seq, 1);
        assert_eq!(session.document().len(), 0);
    }

    #[test]
    fn author_ids_reject_unsafe_text() {
        assert!(AuthorId::new(AuthorKind::Player, "  alex  ").is_ok());
        assert_eq!(AuthorId::player("   "), Err(CommandError::AuthorIdEmpty));
        assert!(matches!(
            AuthorId::agent("bad\nname"),
            Err(CommandError::AuthorIdHasControlChars)
        ));
        assert!(matches!(
            AuthorId::agent("x\u{202E}y"),
            Err(CommandError::AuthorIdHasForbiddenChars)
        ));
        assert_eq!(AuthorId::system().to_string(), "system:system");
    }

    #[test]
    fn round_tower_creation_is_reversible() {
        let mut session = Session::new_empty();
        let before = session.document().document_hash();
        session
            .apply_batch(player(), vec![tower("Drum", [0, 0])])
            .unwrap();
        assert!(matches!(
            session.document().get(1),
            Some(Element::RoundTower(RoundTower {
                radius_mm: 4_000,
                ..
            }))
        ));
        session.undo_last(&player()).unwrap();
        assert_eq!(session.document().document_hash(), before);
    }
}
