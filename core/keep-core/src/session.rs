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
    execute_batch, sanitize_name, uuid_v7, validate_element_budget, AuthorId, AuthorKind,
    CommandError, CommandOp, CommandPayload, InverseRecord, LoggedCommand,
};
use crate::document::{CastleDocument, Element, ElementId, Opening};
use crate::historian::assess_historian;
use crate::junction::junction_partners;
use crate::mesh::{build_scene, SceneData};
use crate::proposal::{
    sanitize_prompt, sanitize_summary, ActivityEntry, AgentParticipant, AgentPermission,
    AgentRequest, AgentState, Proposal, ProposalError, ProposalStatus, RequestError, RequestStatus,
    MAX_ACTIVITY_ENTRIES,
};
use crate::style::StylePack;
use crate::terrain::SiteTerrain;
use crate::validate::validate_structure;

/// Default site id used by the M0a fixtures.
pub const DEFAULT_SITE_ID: &str = "m0a_promontory";

/// Style pack used until the player picks another one.
pub const DEFAULT_STYLE_ID: &str = "edwardian_welsh";

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
    /// Connected agents, in stable id order (architecture section 7.7).
    pub(crate) agents: BTreeMap<String, AgentParticipant>,
    /// Live proposal overlays. Never part of the saved castle (section 4.2).
    pub(crate) proposals: BTreeMap<String, Proposal>,
    /// Deterministic proposal id allocator (`prop_0001`, `prop_0002`, ...).
    pub(crate) next_proposal_seq: u64,
    /// Style pack currently guiding validation and the historian.
    pub(crate) active_style_id: String,
    /// Request inbox, keyed by deterministic request id.
    pub(crate) requests: BTreeMap<String, AgentRequest>,
    /// Deterministic request id allocator (`req_0001`, `req_0002`, ...).
    pub(crate) next_request_seq: u64,
    /// Bounded activity feed, oldest entries evicted first.
    pub(crate) activity: Vec<ActivityEntry>,
    /// Monotonic activity feed sequence.
    pub(crate) next_activity_seq: u64,
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
            agents: BTreeMap::new(),
            proposals: BTreeMap::new(),
            next_proposal_seq: 1,
            active_style_id: DEFAULT_STYLE_ID.to_string(),
            requests: BTreeMap::new(),
            next_request_seq: 1,
            activity: Vec::new(),
            next_activity_seq: 1,
        }
    }

    pub fn site_id(&self) -> &str {
        &self.site_id
    }

    // -----------------------------------------------------------------------
    // Style selection
    // -----------------------------------------------------------------------

    /// Id of the style pack currently guiding validation and the historian.
    pub fn style_id(&self) -> &str {
        &self.active_style_id
    }

    /// Select the active style pack. Unknown ids are ignored and `false` is
    /// returned, leaving the previous selection in place.
    pub fn set_style_id(&mut self, style_id: impl Into<String>) -> bool {
        let style_id = style_id.into();
        if StylePack::find(&style_id).is_some() {
            self.active_style_id = style_id;
            true
        } else {
            false
        }
    }

    /// The active style pack, falling back to the default if the id was
    /// somehow cleared or corrupted.
    fn active_style(&self) -> StylePack {
        StylePack::find(&self.active_style_id).unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // Activity feed
    // -----------------------------------------------------------------------

    /// Append one sanitised entry to the bounded activity feed. The session
    /// clock is read but deliberately not advanced so command UUIDs stay tied
    /// to command-log ticks.
    pub fn record_activity(
        &mut self,
        actor: impl AsRef<str>,
        action: impl AsRef<str>,
        detail: impl AsRef<str>,
    ) {
        let entry = ActivityEntry {
            seq: self.next_activity_seq,
            timestamp_ms: self.clock_ms,
            actor: ActivityEntry::sanitize_field(actor.as_ref()),
            action: ActivityEntry::sanitize_field(action.as_ref()),
            detail: ActivityEntry::sanitize_field(detail.as_ref()),
        };
        self.next_activity_seq += 1;
        self.activity.push(entry);
        if self.activity.len() > MAX_ACTIVITY_ENTRIES {
            let excess = self.activity.len() - MAX_ACTIVITY_ENTRIES;
            self.activity.drain(0..excess);
        }
    }

    /// The activity feed, newest last.
    pub fn activity_feed(&self) -> &[ActivityEntry] {
        &self.activity
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

    // -----------------------------------------------------------------------
    // Agent participants (architecture sections 7.6 and 7.7)
    // -----------------------------------------------------------------------

    /// Register or replace an agent participant. The id uses the same
    /// sanitisation rules as an [`AuthorId`]; the display name is trimmed and
    /// length-limited. An empty or malformed colour falls back to neutral grey.
    pub fn register_agent(
        &mut self,
        id: &str,
        name: &str,
        color_hex: &str,
        permission: AgentPermission,
    ) -> Result<&AgentParticipant, CommandError> {
        let author = AuthorId::agent(id)?;
        let name = sanitize_name(name)?;
        let participant = AgentParticipant {
            id: author.id.clone(),
            name,
            color_hex: sanitize_color_hex(color_hex),
            permission,
            state: AgentState::Idle,
        };
        let display_name = participant.name.clone();
        let agent_id = participant.id.clone();
        self.agents.insert(agent_id.clone(), participant);
        self.record_activity(
            &agent_id,
            "agent_registered",
            format!("{display_name} ({permission:?})"),
        );
        Ok(self
            .agents
            .get(&author.id)
            .expect("agent was inserted immediately above"))
    }

    /// Change an agent's permission. Unknown ids are refused rather than
    /// silently created.
    pub fn set_agent_permission(
        &mut self,
        id: &str,
        permission: AgentPermission,
    ) -> Result<(), ProposalError> {
        let agent = self
            .agents
            .get_mut(id)
            .ok_or(ProposalError::PermissionDenied)?;
        agent.permission = permission;
        self.record_activity(
            "host",
            "agent_permission_changed",
            format!("{id}: {permission:?}"),
        );
        Ok(())
    }

    /// Update the presence state shown in the agent panel. A no-op for unknown
    /// ids so UI presence updates stay infallible.
    pub fn set_agent_state(&mut self, id: &str, state: AgentState) {
        if let Some(agent) = self.agents.get_mut(id) {
            agent.state = state;
        }
    }

    /// Pause an agent. Paused agents are refused by every proposal, command and
    /// request mutation until resumed. Unknown ids are refused.
    pub fn pause_agent(&mut self, id: &str) -> Result<(), ProposalError> {
        let agent = self
            .agents
            .get_mut(id)
            .ok_or(ProposalError::PermissionDenied)?;
        agent.state = AgentState::Paused;
        self.record_activity("host", "agent_paused", id);
        Ok(())
    }

    /// Resume a paused agent. It returns to `WaitingForApproval` when it still
    /// has a submitted proposal, to `BuildingProposal` when it has an unfinished
    /// draft, otherwise to `Idle`. Unknown ids are refused; resuming an
    /// already-active agent is a harmless no-op that still records the action.
    pub fn resume_agent(&mut self, id: &str) -> Result<(), ProposalError> {
        let agent = self
            .agents
            .get_mut(id)
            .ok_or(ProposalError::PermissionDenied)?;
        let has_status = |status: ProposalStatus| {
            self.proposals.values().any(|proposal| {
                proposal.author.kind == AuthorKind::Agent
                    && proposal.author.id == id
                    && proposal.status == status
            })
        };
        agent.state = if has_status(ProposalStatus::Submitted) {
            AgentState::WaitingForApproval
        } else if has_status(ProposalStatus::Draft) {
            AgentState::BuildingProposal
        } else {
            AgentState::Idle
        };
        self.record_activity("host", "agent_resumed", id);
        Ok(())
    }

    /// Disconnect and remove an agent. Returns `true` when an agent was
    /// present.
    pub fn disconnect_agent(&mut self, id: &str) -> bool {
        let removed = self.agents.remove(id).is_some();
        if removed {
            self.record_activity("host", "agent_disconnected", id);
        }
        removed
    }

    /// One agent by sanitised id.
    pub fn agent(&self, id: &str) -> Option<&AgentParticipant> {
        self.agents.get(id)
    }

    /// Connected agents in stable id order.
    pub fn agents(&self) -> impl Iterator<Item = &AgentParticipant> {
        self.agents.values()
    }

    // -----------------------------------------------------------------------
    // Proposals: forked overlays, human-approved (architecture section 4.2)
    // -----------------------------------------------------------------------

    /// Begin a draft proposal against the current document. Read-only and
    /// paused agents are refused (sections 7.6 and 7.7).
    pub fn begin_proposal(
        &mut self,
        author: AuthorId,
        title: &str,
    ) -> Result<Proposal, ProposalError> {
        self.ensure_agent_active(&author, AgentPermission::ProposalOnly)?;
        let title = sanitize_name(title)?;
        let id = format!("prop_{:04}", self.next_proposal_seq);
        self.next_proposal_seq += 1;
        let proposal = Proposal {
            id: id.clone(),
            author,
            title,
            summary: String::new(),
            base_document_hash: document_hash_hex(&self.document),
            base_next_element_id: self.next_element_id,
            ops: Vec::new(),
            created_element_ids: Vec::new(),
            semantic_diff: String::new(),
            structure_before: 0,
            structure_after: 0,
            structure_delta: 0,
            historian_before: 0,
            historian_after: 0,
            historian_delta: 0,
            status: ProposalStatus::Draft,
            reject_reason: None,
        };
        self.proposals.insert(id, proposal.clone());
        if proposal.author.kind == AuthorKind::Agent {
            if let Some(agent) = self.agents.get_mut(&proposal.author.id) {
                agent.state = AgentState::BuildingProposal;
            }
        }
        self.record_activity(
            proposal.author.to_string(),
            "proposal_begun",
            format!("{}: {}", proposal.id, proposal.title),
        );
        Ok(proposal)
    }

    /// Stage additional operations onto a draft proposal. The staged batch is
    /// replayed atomically on a **fork** of the live document to validate it,
    /// compute the semantic diff and structural delta, and record preview ids.
    /// The live document, id allocator and command log are untouched.
    pub fn stage_proposal_ops(
        &mut self,
        proposal_id: &str,
        author: &AuthorId,
        ops: Vec<CommandOp>,
    ) -> Result<Proposal, ProposalError> {
        let proposal = self
            .proposals
            .get(proposal_id)
            .cloned()
            .ok_or(ProposalError::UnknownProposal)?;
        if proposal.author != *author {
            return Err(ProposalError::NotOwner);
        }
        self.ensure_agent_active(author, AgentPermission::ProposalOnly)?;
        if proposal.status != ProposalStatus::Draft {
            return Err(ProposalError::NotDraft);
        }

        let mut combined = proposal.ops.clone();
        combined.extend(ops);
        let application = execute_batch(&self.document, self.next_element_id, &combined)?;

        let style = self.active_style();
        let structure_before = validate_structure(&self.document, &self.terrain, &style).score;
        let structure_after =
            validate_structure(&application.document, &self.terrain, &style).score;
        let historian_before =
            assess_historian(&self.document, &self.terrain, &style).overall_score;
        let historian_after =
            assess_historian(&application.document, &self.terrain, &style).overall_score;

        // Preview ids of everything the proposal adds or modifies. Deleted ids
        // are excluded: they have nothing to render in the ghost overlay.
        let mut created_element_ids: Vec<ElementId> = application
            .touched_elements
            .iter()
            .copied()
            .filter(|id| application.document.get(*id).is_some())
            .collect();
        created_element_ids.sort_unstable();
        created_element_ids.dedup();

        let updated = Proposal {
            ops: combined,
            created_element_ids,
            semantic_diff: application.summary,
            structure_before,
            structure_after,
            structure_delta: structure_after as i32 - structure_before as i32,
            historian_before,
            historian_after,
            historian_delta: historian_after as i32 - historian_before as i32,
            ..proposal
        };
        self.proposals
            .insert(proposal_id.to_string(), updated.clone());
        self.record_activity(
            author.to_string(),
            "proposal_staged",
            format!(
                "{}: {} ops, structure {:+}, historian {:+}",
                updated.id,
                updated.ops.len(),
                updated.structure_delta,
                updated.historian_delta
            ),
        );
        Ok(updated)
    }

    /// Submit a non-empty draft for human approval.
    pub fn submit_proposal(
        &mut self,
        proposal_id: &str,
        author: &AuthorId,
        summary: &str,
    ) -> Result<Proposal, ProposalError> {
        let mut proposal = self
            .proposals
            .get(proposal_id)
            .cloned()
            .ok_or(ProposalError::UnknownProposal)?;
        if proposal.author != *author {
            return Err(ProposalError::NotOwner);
        }
        // A permission downgrade or pause after staging still blocks the
        // submission, not just the earlier lifecycle calls (M1.3 B4).
        self.ensure_agent_active(author, AgentPermission::ProposalOnly)?;
        if proposal.status != ProposalStatus::Draft {
            return Err(ProposalError::NotDraft);
        }
        if proposal.ops.is_empty() {
            return Err(ProposalError::EmptyProposal);
        }
        proposal.summary = sanitize_summary(summary)?;
        proposal.status = ProposalStatus::Submitted;
        self.proposals
            .insert(proposal_id.to_string(), proposal.clone());
        if proposal.author.kind == AuthorKind::Agent {
            if let Some(agent) = self.agents.get_mut(&proposal.author.id) {
                agent.state = AgentState::WaitingForApproval;
            }
        }
        self.record_activity(
            author.to_string(),
            "proposal_submitted",
            format!("{}: {}", proposal.id, proposal.summary),
        );
        Ok(proposal)
    }

    /// Accept a submitted proposal as one atomic batch attributed to the
    /// agent. This is the human approval step: it is deliberately only exposed
    /// on the host/session, never to the proposal-only agent surface. Refuses
    /// with [`ProposalError::StaleBase`] when the castle changed underneath.
    pub fn accept_proposal(&mut self, proposal_id: &str) -> Result<ApplyOutcome, ProposalError> {
        let mut proposal = self
            .proposals
            .get(proposal_id)
            .cloned()
            .ok_or(ProposalError::UnknownProposal)?;
        if proposal.status != ProposalStatus::Submitted {
            return Err(ProposalError::NotSubmitted);
        }
        // The id allocator is monotonic, so comparing it with the recorded
        // value in addition to the document hash catches delete-and-recreate
        // cycles that leave the hash unchanged (M1.3 Medium #1).
        if proposal.base_document_hash != document_hash_hex(&self.document)
            || proposal.base_next_element_id != self.next_element_id
        {
            return Err(ProposalError::StaleBase);
        }

        let outcome = self.apply_batch(proposal.author.clone(), proposal.ops.clone())?;
        proposal.status = ProposalStatus::Accepted;
        self.proposals
            .insert(proposal_id.to_string(), proposal.clone());
        if proposal.author.kind == AuthorKind::Agent {
            if let Some(agent) = self.agents.get_mut(&proposal.author.id) {
                agent.state = AgentState::Idle;
            }
        }
        self.record_activity(
            "host",
            "proposal_accepted",
            format!("{}: {} ops applied", proposal.id, proposal.ops.len()),
        );
        Ok(outcome)
    }

    /// Reject a submitted proposal. The live document and command log stay
    /// exactly as they were; only the overlay records the decision.
    pub fn reject_proposal(
        &mut self,
        proposal_id: &str,
        reason: Option<&str>,
    ) -> Result<Proposal, ProposalError> {
        let mut proposal = self
            .proposals
            .get(proposal_id)
            .cloned()
            .ok_or(ProposalError::UnknownProposal)?;
        if proposal.status != ProposalStatus::Submitted {
            return Err(ProposalError::NotSubmitted);
        }
        let reason = match reason {
            Some(reason) => Some(sanitize_summary(reason)?),
            None => None,
        };
        proposal.status = ProposalStatus::Rejected;
        proposal.reject_reason = reason;
        self.proposals
            .insert(proposal_id.to_string(), proposal.clone());
        if proposal.author.kind == AuthorKind::Agent {
            if let Some(agent) = self.agents.get_mut(&proposal.author.id) {
                agent.state = AgentState::Idle;
            }
        }
        let detail = match &proposal.reject_reason {
            Some(reason) => format!("{}: {reason}", proposal.id),
            None => proposal.id.clone(),
        };
        self.record_activity("host", "proposal_rejected", detail);
        Ok(proposal)
    }

    /// All proposals in stable id order.
    pub fn proposals(&self) -> impl Iterator<Item = &Proposal> {
        self.proposals.values()
    }

    /// One proposal by id.
    pub fn proposal(&self, id: &str) -> Option<&Proposal> {
        self.proposals.get(id)
    }

    /// True when the named proposal exists and the live document has moved
    /// since it was begun. Unknown proposals are not stale (they are simply
    /// absent).
    pub fn is_proposal_stale(&self, proposal_id: &str) -> bool {
        let Some(proposal) = self.proposals.get(proposal_id) else {
            return false;
        };
        proposal.base_document_hash != document_hash_hex(&self.document)
            || proposal.base_next_element_id != self.next_element_id
    }

    /// Build a translucent-overlay scene for a proposal: only the elements the
    /// proposal adds or modifies, taken from a fresh fork of the live document.
    /// Unchanged castle elements are deliberately omitted so the ghost cannot
    /// double-draw the real castle. Returns `None` for unknown proposals or
    /// when the staged operations no longer replay cleanly.
    pub fn build_proposal_ghost_scene(&self, proposal_id: &str) -> Option<SceneData> {
        let proposal = self.proposals.get(proposal_id)?;
        // Only live overlays (draft or awaiting approval) may draw a ghost.
        // Accepted, rejected or empty proposals have nothing to preview
        // (M1.3 Medium #3).
        if !matches!(
            proposal.status,
            ProposalStatus::Draft | ProposalStatus::Submitted
        ) || proposal.ops.is_empty()
        {
            return None;
        }
        let application =
            execute_batch(&self.document, self.next_element_id, &proposal.ops).ok()?;

        let mut ghost = CastleDocument::new();
        for id in &proposal.created_element_ids {
            if let Some(element) = application.document.get(*id) {
                ghost.insert(element.clone());
            }
        }
        // Openings are voids inside their wall, so they only matter when their
        // parent wall is part of the ghost.
        for id in &proposal.created_element_ids {
            for opening in application.document.openings_for_wall(*id) {
                ghost.insert_opening(opening.clone());
            }
        }
        Some(build_scene(&ghost, &self.terrain))
    }

    // -----------------------------------------------------------------------
    // Request inbox (architecture section 7.7)
    // -----------------------------------------------------------------------

    /// Submit a request to the inbox. The prompt is sanitised and bounded; an
    /// optional linked proposal must already exist.
    pub fn submit_request(
        &mut self,
        author: AuthorId,
        prompt: &str,
        proposal_id: Option<&str>,
        pinned_element_id: Option<ElementId>,
    ) -> Result<AgentRequest, RequestError> {
        let prompt = sanitize_prompt(prompt)?;
        if let Some(id) = proposal_id {
            if !self.proposals.contains_key(id) {
                return Err(RequestError::UnknownRequest);
            }
        }
        let seq = self.next_request_seq;
        let request = AgentRequest {
            id: format!("req_{seq:04}"),
            author,
            prompt,
            proposal_id: proposal_id.map(str::to_string),
            pinned_element_id,
            status: RequestStatus::Open,
            claimed_by: None,
            reply: None,
            created_seq: seq,
        };
        self.next_request_seq += 1;
        self.requests.insert(request.id.clone(), request.clone());
        self.record_activity(
            request.author.to_string(),
            "request_submitted",
            format!("{}: {}", request.id, request.prompt),
        );
        Ok(request)
    }

    /// Ask for changes to a submitted proposal, queuing a request linked to it.
    pub fn ask_for_changes(
        &mut self,
        proposal_id: &str,
        author: AuthorId,
        feedback: &str,
    ) -> Result<AgentRequest, RequestError> {
        let proposal = self
            .proposals
            .get(proposal_id)
            .ok_or(RequestError::UnknownRequest)?;
        if proposal.status != ProposalStatus::Submitted {
            return Err(RequestError::InvalidStatus);
        }
        let request = self.submit_request(author, feedback, Some(proposal_id), None)?;
        self.record_activity(
            request.author.to_string(),
            "changes_requested",
            format!("{} -> {proposal_id}", request.id),
        );
        Ok(request)
    }

    /// Claim an open request. Requires an active agent with at least
    /// [`AgentPermission::ReadOnly`].
    pub fn ack_request(
        &mut self,
        request_id: &str,
        agent_id: &str,
    ) -> Result<AgentRequest, RequestError> {
        let author = AuthorId::agent(agent_id).map_err(RequestError::Command)?;
        self.ensure_agent_active(&author, AgentPermission::ReadOnly)
            .map_err(|_| RequestError::PermissionDenied)?;
        let request = self
            .requests
            .get_mut(request_id)
            .ok_or(RequestError::UnknownRequest)?;
        if request.status != RequestStatus::Open {
            return Err(RequestError::InvalidStatus);
        }
        request.status = RequestStatus::Acknowledged;
        request.claimed_by = Some(agent_id.to_string());
        let updated = request.clone();
        self.record_activity(
            format!("agent:{agent_id}"),
            "request_acknowledged",
            &updated.id,
        );
        Ok(updated)
    }

    /// Resolve an open or acknowledged request, optionally recording a reply.
    /// Requires an active agent with at least [`AgentPermission::ReadOnly`].
    pub fn resolve_request(
        &mut self,
        request_id: &str,
        agent_id: &str,
        reply: Option<&str>,
    ) -> Result<AgentRequest, RequestError> {
        let author = AuthorId::agent(agent_id).map_err(RequestError::Command)?;
        self.ensure_agent_active(&author, AgentPermission::ReadOnly)
            .map_err(|_| RequestError::PermissionDenied)?;
        let reply = match reply {
            Some(reply) => Some(sanitize_prompt(reply)?),
            None => None,
        };
        let request = self
            .requests
            .get_mut(request_id)
            .ok_or(RequestError::UnknownRequest)?;
        if !matches!(
            request.status,
            RequestStatus::Open | RequestStatus::Acknowledged
        ) {
            return Err(RequestError::InvalidStatus);
        }
        request.status = RequestStatus::Resolved;
        if request.claimed_by.is_none() {
            request.claimed_by = Some(agent_id.to_string());
        }
        request.reply = reply;
        let updated = request.clone();
        self.record_activity(format!("agent:{agent_id}"), "request_resolved", &updated.id);
        Ok(updated)
    }

    /// All requests in deterministic id order.
    pub fn requests(&self) -> impl Iterator<Item = &AgentRequest> {
        self.requests.values()
    }

    /// Open and acknowledged requests in deterministic id order.
    pub fn open_requests(&self) -> Vec<&AgentRequest> {
        self.requests
            .values()
            .filter(|request| {
                matches!(
                    request.status,
                    RequestStatus::Open | RequestStatus::Acknowledged
                )
            })
            .collect()
    }

    /// One request by id.
    pub fn request(&self, id: &str) -> Option<&AgentRequest> {
        self.requests.get(id)
    }

    /// Shared gate for every agent-facing mutation: the author must be a
    /// registered agent holding *at least* `required` permission and must not
    /// be paused. Player and system authors always pass (M1.3 B4).
    pub fn ensure_agent_active(
        &self,
        author: &AuthorId,
        required: AgentPermission,
    ) -> Result<(), ProposalError> {
        if author.kind != AuthorKind::Agent {
            return Ok(());
        }
        let agent = self
            .agents
            .get(&author.id)
            .ok_or(ProposalError::PermissionDenied)?;
        if agent.state == AgentState::Paused {
            return Err(ProposalError::PermissionDenied);
        }
        if agent.permission.level() < required.level() {
            return Err(ProposalError::PermissionDenied);
        }
        Ok(())
    }
}

/// Hex document hash used for proposal base checks.
fn document_hash_hex(document: &CastleDocument) -> String {
    format!("{:016x}", document.document_hash())
}

/// Keep a colour readable by the UI and free of control characters.
fn sanitize_color_hex(value: &str) -> String {
    let trimmed = value.trim();
    let valid = !trimmed.is_empty()
        && trimmed.len() <= 16
        && trimmed.chars().all(|c| c == '#' || c.is_ascii_hexdigit());
    if valid {
        trimmed.to_string()
    } else {
        "#888888".to_string()
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

    #[test]
    fn modify_element_changes_and_undoes_wall_and_tower_plan() {
        let mut session = Session::new_empty();
        session
            .apply_batch(player(), vec![wall("North", [0, 0], [10_000, 0])])
            .unwrap();
        session
            .apply_batch(player(), vec![tower("Drum", [20_000, 0])])
            .unwrap();
        let before_hash = session.document().document_hash();
        let before_layout = session.build_scene().layout_hash;

        session
            .apply_batch(
                player(),
                vec![
                    CommandOp::ModifyElement {
                        id: 1,
                        height_mm: Some(11_000),
                        thickness_mm: Some(3_200),
                        batter_mm: Some(1_100),
                        plinth_height_mm: Some(1_000),
                        plan: None,
                        orientation_deg: None,
                    },
                    CommandOp::ModifyElement {
                        id: 2,
                        height_mm: Some(16_000),
                        thickness_mm: Some(1_500),
                        batter_mm: None,
                        plinth_height_mm: None,
                        plan: Some(crate::document::TowerPlan::Square),
                        orientation_deg: Some(45),
                    },
                ],
            )
            .unwrap();

        let Some(Element::BatteredWall(wall)) = session.document().get(1) else {
            panic!("wall missing");
        };
        assert_eq!(wall.height_mm, 11_000);
        assert_eq!(wall.thickness_mm, 3_200);
        let Some(Element::RoundTower(tower)) = session.document().get(2) else {
            panic!("tower missing");
        };
        assert_eq!(tower.plan, crate::document::TowerPlan::Square);
        assert_eq!(tower.orientation_deg, 45);
        assert_ne!(session.build_scene().layout_hash, before_layout);

        session.undo_last(&player()).unwrap();
        assert_eq!(session.document().document_hash(), before_hash);
        assert_eq!(session.build_scene().layout_hash, before_layout);
        let Some(Element::RoundTower(tower)) = session.document().get(2) else {
            panic!("tower missing");
        };
        assert_eq!(tower.plan, crate::document::TowerPlan::Round);
    }

    #[test]
    fn resume_restores_building_state_for_an_unfinished_draft() {
        let mut session = Session::new_empty();
        session
            .register_agent("mason", "Mason", "#e8b04b", AgentPermission::ProposalOnly)
            .unwrap();
        let author = agent();
        let proposal = session
            .begin_proposal(author.clone(), "Flanking tower")
            .unwrap();
        session
            .stage_proposal_ops(&proposal.id, &author, vec![tower("Drum", [0, 0])])
            .unwrap();
        session.pause_agent("mason").unwrap();
        assert_eq!(session.agent("mason").unwrap().state, AgentState::Paused);

        session.resume_agent("mason").unwrap();
        assert_eq!(
            session.agent("mason").unwrap().state,
            AgentState::BuildingProposal,
            "a paused agent with an unfinished draft resumes mid-build"
        );
    }

    #[test]
    fn request_activity_logs_the_agent_actor_prefix() {
        let mut session = Session::new_empty();
        session
            .register_agent("mason", "Mason", "#e8b04b", AgentPermission::ReadOnly)
            .unwrap();
        let request = session
            .submit_request(player(), "Please add a tower", None, None)
            .unwrap();
        session.ack_request(&request.id, "mason").unwrap();
        session
            .resolve_request(&request.id, "mason", Some("Done"))
            .unwrap();

        let actors: Vec<_> = session
            .activity_feed()
            .iter()
            .filter(|entry| {
                matches!(
                    entry.action.as_str(),
                    "request_acknowledged" | "request_resolved"
                )
            })
            .map(|entry| entry.actor.as_str())
            .collect();
        assert_eq!(actors, vec!["agent:mason", "agent:mason"]);
    }

    #[test]
    fn ditch_lowers_the_built_terrain_and_undo_restores_it() {
        let mut session = Session::new_empty();
        let before = session.build_scene();
        let ground = session.terrain().height_mm(0, 0);
        session
            .apply_batch(
                player(),
                vec![CommandOp::CreateDitch {
                    ref_id: Some("approach".into()),
                    name: "Approach ditch".into(),
                    start_mm: [0, -8_000],
                    end_mm: [0, 8_000],
                    width_mm: 4_000,
                    depth_mm: 2_000,
                }],
            )
            .unwrap();
        let carved = session.build_scene();
        assert!(carved.terrain_mesh.positions.iter().any(|position| {
            position[0].abs() < 0.01
                && position[2].abs() < 0.01
                && (position[1] - (ground - 2_000) as f32 / 1000.0).abs() < 0.2
        }));

        session.undo_last(&player()).unwrap();
        let restored = session.build_scene();
        assert_eq!(restored.layout_hash, before.layout_hash);
        assert_eq!(
            restored.terrain_mesh.positions,
            before.terrain_mesh.positions
        );
        assert!(session.document().ditches().next().is_none());
    }
}
