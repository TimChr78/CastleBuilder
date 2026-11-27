//! Proposal overlay system and agent participants.
//!
//! Proposals are **live session overlays**, never part of the [`CastleDocument`]
//! or the saved castle: an agent forks the document in memory, stages a batch
//! of semantic operations against the fork, and submits a diff for a human to
//! approve. Accepting a proposal applies its operations to the live document as
//! one attributed atomic batch, exactly as if the agent had used the command
//! bus directly. Rejecting one leaves the live document and command log
//! untouched (architecture sections 4.2, 7.6, 7.7 and 13 M1).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::command::{AuthorId, CommandError, CommandOp};
use crate::document::ElementId;

/// Maximum number of characters in a proposal summary or reject reason.
pub const MAX_SUMMARY_CHARS: usize = 256;

/// What an agent participant is allowed to do (architecture section 7.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPermission {
    /// May read the document, assessment and renders only.
    ReadOnly,
    /// May stage proposals, but only a human can accept them. The default for
    /// every newly connected agent.
    #[default]
    ProposalOnly,
    /// May commit commands directly through the command bus.
    DirectEdit,
}

impl AgentPermission {
    /// Position in the permission hierarchy:
    /// `ReadOnly < ProposalOnly < DirectEdit`. Used to check whether an agent
    /// holds at least the level an operation requires.
    pub fn level(self) -> u8 {
        match self {
            Self::ReadOnly => 0,
            Self::ProposalOnly => 1,
            Self::DirectEdit => 2,
        }
    }
}

/// The in-game presence state shown in the agent panel (section 7.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Idle,
    Thinking,
    BuildingProposal,
    WaitingForApproval,
    /// A paused agent's proposals and commands are refused until resumed.
    Paused,
}

/// One connected agent, surfaced in the in-game agent panel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentParticipant {
    /// Sanitised stable identifier (the same text used in `AuthorId::agent`).
    pub id: String,
    /// Sanitised display name.
    pub name: String,
    /// UI colour, for example `#4fa3ff`.
    pub color_hex: String,
    pub permission: AgentPermission,
    pub state: AgentState,
}

/// Lifecycle of a proposal overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    /// The agent is still staging operations.
    Draft,
    /// Submitted for human approval.
    Submitted,
    /// Accepted and applied to the live document as one atomic batch.
    Accepted,
    /// Refused by the human; the live document is untouched.
    Rejected,
}

/// A staged, human-approvable diff against a fork of the live document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    /// Deterministic `"prop_0001"`, `"prop_0002"`, ... identifier.
    pub id: String,
    pub author: AuthorId,
    /// Sanitised human-readable title.
    pub title: String,
    /// Sanitised short summary; at most [`MAX_SUMMARY_CHARS`] characters and
    /// free of control and bidi characters.
    pub summary: String,
    /// Hex hash of [`crate::session::Session::document`] when the proposal was
    /// begun. Acceptance refuses with [`ProposalError::StaleBase`] when the
    /// castle changed underneath it.
    pub base_document_hash: String,
    /// Value of [`crate::session::Session::next_element_id`] when the proposal
    /// was begun. Acceptance refuses with [`ProposalError::StaleBase`] when the
    /// allocator moved, even if the document hash happens to match (for
    /// example after a delete-and-recreate cycle).
    pub base_next_element_id: ElementId,
    /// The complete staged operation batch, replayed from the base document.
    pub ops: Vec<CommandOp>,
    /// Preview element ids created or modified by the staged batch on the
    /// forked document. Used to build the ghost overlay.
    pub created_element_ids: Vec<ElementId>,
    /// Semantic diff produced while staging (for example `+1 wall, +1 tower`).
    pub semantic_diff: String,
    /// Structural score of the base document.
    pub structure_before: u32,
    /// Structural score of the forked document with the proposal applied.
    pub structure_after: u32,
    /// `structure_after - structure_before`.
    pub structure_delta: i32,
    pub status: ProposalStatus,
    /// Sanitised reason supplied when a human rejected the proposal.
    pub reject_reason: Option<String>,
}

impl Proposal {
    /// True while the author may still stage operations.
    pub fn is_draft(&self) -> bool {
        self.status == ProposalStatus::Draft
    }
}

/// Why a proposal operation was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalError {
    UnknownProposal,
    /// The proposal is not in [`ProposalStatus::Draft`].
    NotDraft,
    /// The proposal is not in [`ProposalStatus::Submitted`].
    NotSubmitted,
    /// The caller does not own (author) this proposal.
    NotOwner,
    /// The agent's permission or paused state forbids the operation.
    PermissionDenied,
    /// The live document changed since the proposal was begun.
    StaleBase,
    /// A proposal cannot be submitted without staged operations.
    EmptyProposal,
    /// A staged operation failed validation or application.
    Command(CommandError),
}

impl fmt::Display for ProposalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownProposal => write!(f, "unknown proposal"),
            Self::NotDraft => write!(f, "proposal is not a draft"),
            Self::NotSubmitted => write!(f, "proposal has not been submitted"),
            Self::NotOwner => write!(f, "proposal belongs to another author"),
            Self::PermissionDenied => write!(f, "agent permission or paused state forbids this"),
            Self::StaleBase => write!(f, "the castle changed since this proposal was begun"),
            Self::EmptyProposal => write!(f, "proposal has no staged operations"),
            Self::Command(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ProposalError {}

impl From<CommandError> for ProposalError {
    fn from(error: CommandError) -> Self {
        Self::Command(error)
    }
}

/// Trim and sanitise a proposal summary or reject reason (architecture section
/// 7.6): at most [`MAX_SUMMARY_CHARS`] characters and no control, bidi,
/// formatting, separator or private-use characters.
pub fn sanitize_summary(summary: &str) -> Result<String, CommandError> {
    let trimmed = summary.trim();
    if trimmed.chars().any(char::is_control) {
        return Err(CommandError::NameHasControlChars);
    }
    if trimmed.chars().any(is_forbidden_display_char) {
        return Err(CommandError::NameHasForbiddenChars);
    }
    let chars = trimmed.chars().count();
    if chars > MAX_SUMMARY_CHARS {
        return Err(CommandError::NameTooLong {
            chars,
            max: MAX_SUMMARY_CHARS,
        });
    }
    Ok(trimmed.to_string())
}

fn is_forbidden_display_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{2028}' | '\u{2029}')
        || matches!(
            c as u32,
            0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFEFF
        )
        || matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Element;
    use crate::session::Session;

    fn agent_id() -> AuthorId {
        AuthorId::agent("mason").unwrap()
    }

    fn player_id() -> AuthorId {
        AuthorId::player("alex").unwrap()
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

    fn session_with_agent(permission: AgentPermission) -> Session {
        let mut session = Session::new_empty();
        session
            .register_agent("mason", "Mason", "#4fa3ff", permission)
            .unwrap();
        session
    }

    #[test]
    fn full_flow_leaves_the_live_document_untouched_until_accept() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);
        let before_hash = session.document().document_hash();
        let before_layout = session.build_scene().layout_hash;

        let proposal = session.begin_proposal(agent_id(), "North curtain").unwrap();
        assert_eq!(proposal.id, "prop_0001");
        assert_eq!(proposal.status, ProposalStatus::Draft);
        assert_eq!(proposal.base_document_hash, format!("{before_hash:016x}"));
        assert_eq!(proposal.base_next_element_id, session.next_element_id());
        assert_eq!(
            session.agent("mason").unwrap().state,
            AgentState::BuildingProposal
        );

        let proposal = session
            .stage_proposal_ops(
                &proposal.id,
                &agent_id(),
                vec![wall("North curtain", [-8_000, 0], [8_000, 0])],
            )
            .unwrap();
        assert_eq!(proposal.ops.len(), 1);
        assert_eq!(proposal.created_element_ids.len(), 1);
        assert!(!proposal.semantic_diff.is_empty());
        assert_eq!(session.document().document_hash(), before_hash);

        let proposal = session
            .submit_proposal(&proposal.id, &agent_id(), "A new north curtain")
            .unwrap();
        assert_eq!(proposal.status, ProposalStatus::Submitted);
        assert_eq!(proposal.summary, "A new north curtain");
        assert_eq!(
            session.agent("mason").unwrap().state,
            AgentState::WaitingForApproval
        );
        assert_eq!(session.document().document_hash(), before_hash);
        assert_eq!(session.build_scene().layout_hash, before_layout);
        assert!(session.history().is_empty());

        let outcome = session.accept_proposal(&proposal.id).unwrap();
        assert_eq!(outcome.seq, 1);
        assert_eq!(session.history().len(), 1);
        assert_eq!(session.history()[0].author, agent_id());
        assert_ne!(session.document().document_hash(), before_hash);
        assert_eq!(session.document().len(), 1);
        assert_eq!(
            session.proposal(&proposal.id).unwrap().status,
            ProposalStatus::Accepted
        );
        assert_eq!(session.agent("mason").unwrap().state, AgentState::Idle);

        // The accepted batch is attributed to the agent and can be undone by it.
        let undo = session.undo_last(&agent_id()).unwrap();
        assert_eq!(undo.target_seq, 1);
        assert_eq!(session.document().document_hash(), before_hash);
        assert_eq!(session.build_scene().layout_hash, before_layout);
    }

    #[test]
    fn proposal_ids_are_deterministic_and_monotonic() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);
        assert_eq!(
            session.begin_proposal(agent_id(), "One").unwrap().id,
            "prop_0001"
        );
        assert_eq!(
            session.begin_proposal(agent_id(), "Two").unwrap().id,
            "prop_0002"
        );
        assert_eq!(
            session.begin_proposal(agent_id(), "Three").unwrap().id,
            "prop_0003"
        );
    }

    #[test]
    fn reject_leaves_the_document_and_log_untouched() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);
        let before_hash = session.document().document_hash();
        let proposal = session.begin_proposal(agent_id(), "Tower").unwrap();
        session
            .stage_proposal_ops(&proposal.id, &agent_id(), vec![tower("Drum", [10_000, 0])])
            .unwrap();
        session
            .submit_proposal(&proposal.id, &agent_id(), "A drum tower")
            .unwrap();

        let rejected = session
            .reject_proposal(&proposal.id, Some("Too close to the gate"))
            .unwrap();
        assert_eq!(rejected.status, ProposalStatus::Rejected);
        assert_eq!(
            rejected.reject_reason.as_deref(),
            Some("Too close to the gate")
        );
        assert_eq!(session.document().document_hash(), before_hash);
        assert!(session.document().is_empty());
        assert!(session.history().is_empty());
        assert_eq!(session.agent("mason").unwrap().state, AgentState::Idle);

        // A rejected proposal cannot be accepted afterwards.
        assert_eq!(
            session.accept_proposal(&proposal.id),
            Err(ProposalError::NotSubmitted)
        );
    }

    #[test]
    fn reject_reason_is_sanitised_and_length_limited() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);
        let proposal = session.begin_proposal(agent_id(), "Tower").unwrap();
        session
            .stage_proposal_ops(&proposal.id, &agent_id(), vec![tower("Drum", [10_000, 0])])
            .unwrap();
        session
            .submit_proposal(&proposal.id, &agent_id(), "A drum tower")
            .unwrap();
        let too_long = "x".repeat(MAX_SUMMARY_CHARS + 1);
        assert!(matches!(
            session.reject_proposal(&proposal.id, Some(&too_long)),
            Err(ProposalError::Command(CommandError::NameTooLong { .. }))
        ));

        let rejected = session
            .reject_proposal(&proposal.id, Some("  A bad idea  "))
            .unwrap();
        assert_eq!(rejected.reject_reason.as_deref(), Some("A bad idea"));
    }

    #[test]
    fn a_stale_base_refuses_acceptance() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);
        let proposal = session.begin_proposal(agent_id(), "North curtain").unwrap();
        session
            .stage_proposal_ops(
                &proposal.id,
                &agent_id(),
                vec![wall("North curtain", [-8_000, 0], [8_000, 0])],
            )
            .unwrap();
        session
            .submit_proposal(&proposal.id, &agent_id(), "A new north curtain")
            .unwrap();

        // The player edits the castle while the proposal waits for approval.
        session
            .apply_batch(player_id(), vec![tower("Player drum", [30_000, 0])])
            .unwrap();
        let after_player_hash = session.document().document_hash();

        assert_eq!(
            session.accept_proposal(&proposal.id),
            Err(ProposalError::StaleBase)
        );
        assert_eq!(session.document().document_hash(), after_player_hash);
        assert_eq!(session.document().len(), 1);
        assert_eq!(session.history().len(), 1);
        assert_eq!(
            session.proposal(&proposal.id).unwrap().status,
            ProposalStatus::Submitted
        );
    }

    #[test]
    fn read_only_and_paused_agents_cannot_begin_a_proposal() {
        let mut session = session_with_agent(AgentPermission::ReadOnly);
        assert_eq!(
            session.begin_proposal(agent_id(), "Nope"),
            Err(ProposalError::PermissionDenied)
        );

        session
            .set_agent_permission("mason", AgentPermission::ProposalOnly)
            .unwrap();
        session.set_agent_state("mason", AgentState::Paused);
        assert_eq!(
            session.begin_proposal(agent_id(), "Nope"),
            Err(ProposalError::PermissionDenied)
        );

        session.set_agent_state("mason", AgentState::Idle);
        assert!(session.begin_proposal(agent_id(), "Yes").is_ok());
    }

    #[test]
    fn ownership_is_enforced_when_staging_and_submitting() {
        let mut session = Session::new_empty();
        session
            .register_agent("mason", "Mason", "#4fa3ff", AgentPermission::ProposalOnly)
            .unwrap();
        session
            .register_agent("wright", "Wright", "#ffa34f", AgentPermission::ProposalOnly)
            .unwrap();
        let proposal = session.begin_proposal(agent_id(), "Wall").unwrap();
        let other = AuthorId::agent("wright").unwrap();

        assert_eq!(
            session.stage_proposal_ops(&proposal.id, &other, vec![tower("T", [0, 0])]),
            Err(ProposalError::NotOwner)
        );
        session
            .stage_proposal_ops(&proposal.id, &agent_id(), vec![tower("T", [0, 0])])
            .unwrap();
        assert_eq!(
            session.submit_proposal(&proposal.id, &other, "Mine now"),
            Err(ProposalError::NotOwner)
        );
    }

    #[test]
    fn an_empty_proposal_cannot_be_submitted_and_bad_ops_do_not_stick() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);
        let proposal = session.begin_proposal(agent_id(), "Nothing").unwrap();
        assert_eq!(
            session.submit_proposal(&proposal.id, &agent_id(), "Nothing"),
            Err(ProposalError::EmptyProposal)
        );

        let error = session
            .stage_proposal_ops(
                &proposal.id,
                &agent_id(),
                vec![CommandOp::DeleteElement { id: 999 }],
            )
            .unwrap_err();
        assert_eq!(
            error,
            ProposalError::Command(CommandError::UnknownElementId(999))
        );
        assert!(session.proposal(&proposal.id).unwrap().ops.is_empty());
        assert!(session.document().is_empty());
    }

    #[test]
    fn ghost_scene_contains_only_the_proposed_elements() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);
        // An existing castle element the proposal must not duplicate.
        session
            .apply_batch(
                player_id(),
                vec![wall("West curtain", [-20_000, 0], [-8_000, 0])],
            )
            .unwrap();

        let proposal = session.begin_proposal(agent_id(), "Drum tower").unwrap();
        session
            .stage_proposal_ops(
                &proposal.id,
                &agent_id(),
                vec![tower("Drum tower", [10_000, 0])],
            )
            .unwrap();
        session
            .submit_proposal(&proposal.id, &agent_id(), "Add a drum tower")
            .unwrap();

        let ghost = session.build_proposal_ghost_scene(&proposal.id).unwrap();
        assert!(!ghost.chunks.is_empty(), "ghost must produce render chunks");
        assert!(ghost.block_count > 0);
        let ghost_blocks = ghost.block_count;
        let ghost_layout = ghost.layout_hash;

        // Accept, then compare against the whole castle: the ghost must be a
        // strict subset, proving it did not duplicate unchanged elements.
        session.accept_proposal(&proposal.id).unwrap();
        let full = session.build_scene();
        assert!(ghost_blocks < full.block_count);
        assert_ne!(ghost_layout, full.layout_hash);

        let scene_for_tower = {
            let mut doc = crate::document::CastleDocument::new();
            doc.insert(
                session
                    .document()
                    .elements()
                    .find(|element| element.name() == "Drum tower")
                    .cloned()
                    .unwrap(),
            );
            crate::mesh::build_scene(&doc, session.terrain())
        };
        assert_eq!(ghost_blocks, scene_for_tower.block_count);
    }

    #[test]
    fn ghost_for_unknown_or_draft_empty_proposal_is_empty() {
        let session = Session::new_empty();
        assert!(session.build_proposal_ghost_scene("prop_9999").is_none());
    }

    #[test]
    fn agents_are_registered_sanitised_and_listed_in_order() {
        let mut session = Session::new_empty();
        session
            .register_agent(
                "  mason  ",
                "  Mason  ",
                "#4fa3ff",
                AgentPermission::ReadOnly,
            )
            .unwrap();
        session
            .register_agent("wright", "Wright", "#ffa34f", AgentPermission::DirectEdit)
            .unwrap();
        let ids: Vec<_> = session.agents().map(|agent| agent.id.as_str()).collect();
        assert_eq!(ids, vec!["mason", "wright"]);
        assert_eq!(session.agent("mason").unwrap().name, "Mason");
        assert_eq!(
            session.agent("mason").unwrap().permission,
            AgentPermission::ReadOnly
        );
        assert!(session
            .register_agent("bad\nname", "X", "#fff", AgentPermission::ReadOnly)
            .is_err());
    }

    #[test]
    fn permission_and_state_updates_apply_to_known_agents() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);
        session
            .set_agent_permission("mason", AgentPermission::DirectEdit)
            .unwrap();
        assert_eq!(
            session.agent("mason").unwrap().permission,
            AgentPermission::DirectEdit
        );
        session.set_agent_state("mason", AgentState::Thinking);
        assert_eq!(session.agent("mason").unwrap().state, AgentState::Thinking);
        assert_eq!(
            session.set_agent_permission("ghost", AgentPermission::ReadOnly),
            Err(ProposalError::PermissionDenied)
        );
        // Unknown ids are a no-op rather than a panic.
        session.set_agent_state("ghost", AgentState::Paused);
    }

    #[test]
    fn accepted_proposal_elements_are_real_document_elements() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);
        let proposal = session.begin_proposal(agent_id(), "Wall").unwrap();
        session
            .stage_proposal_ops(
                &proposal.id,
                &agent_id(),
                vec![wall("Proposed wall", [0, 0], [12_000, 0])],
            )
            .unwrap();
        session
            .submit_proposal(&proposal.id, &agent_id(), "Wall")
            .unwrap();
        session.accept_proposal(&proposal.id).unwrap();

        let ids: Vec<ElementId> = session.document().elements().map(Element::id).collect();
        assert_eq!(ids, vec![1]);
    }

    #[test]
    fn ensure_agent_active_checks_registration_pause_and_permission_level() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);
        assert!(session
            .ensure_agent_active(&agent_id(), AgentPermission::ProposalOnly)
            .is_ok());
        // ProposalOnly is below DirectEdit.
        assert_eq!(
            session.ensure_agent_active(&agent_id(), AgentPermission::DirectEdit),
            Err(ProposalError::PermissionDenied)
        );

        // A paused agent is refused even when it holds a high permission.
        session
            .set_agent_permission("mason", AgentPermission::DirectEdit)
            .unwrap();
        session.set_agent_state("mason", AgentState::Paused);
        assert_eq!(
            session.ensure_agent_active(&agent_id(), AgentPermission::DirectEdit),
            Err(ProposalError::PermissionDenied)
        );

        // An unregistered agent id is refused.
        let ghost = AuthorId::agent("ghost").unwrap();
        assert_eq!(
            session.ensure_agent_active(&ghost, AgentPermission::ReadOnly),
            Err(ProposalError::PermissionDenied)
        );

        // Players and the system are never gated by agent permissions.
        assert!(session
            .ensure_agent_active(&player_id(), AgentPermission::DirectEdit)
            .is_ok());
        assert!(session
            .ensure_agent_active(&AuthorId::system(), AgentPermission::DirectEdit)
            .is_ok());
    }

    #[test]
    fn paused_agent_cannot_submit_a_staged_proposal() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);
        let proposal = session.begin_proposal(agent_id(), "Wall").unwrap();
        session
            .stage_proposal_ops(
                &proposal.id,
                &agent_id(),
                vec![wall("Proposed wall", [0, 0], [12_000, 0])],
            )
            .unwrap();

        session.set_agent_state("mason", AgentState::Paused);
        assert_eq!(
            session.submit_proposal(&proposal.id, &agent_id(), "Wall"),
            Err(ProposalError::PermissionDenied)
        );
        assert_eq!(
            session.proposal(&proposal.id).unwrap().status,
            ProposalStatus::Draft
        );
    }

    #[test]
    fn demoted_agent_cannot_submit_a_staged_proposal() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);
        let proposal = session.begin_proposal(agent_id(), "Wall").unwrap();
        session
            .stage_proposal_ops(
                &proposal.id,
                &agent_id(),
                vec![wall("Proposed wall", [0, 0], [12_000, 0])],
            )
            .unwrap();

        // The human demotes the agent after it staged its work.
        session
            .set_agent_permission("mason", AgentPermission::ReadOnly)
            .unwrap();
        assert_eq!(
            session.submit_proposal(&proposal.id, &agent_id(), "Wall"),
            Err(ProposalError::PermissionDenied)
        );
        assert_eq!(
            session.proposal(&proposal.id).unwrap().status,
            ProposalStatus::Draft
        );
    }

    #[test]
    fn a_moved_id_allocator_refuses_acceptance_even_when_hashes_match() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);
        let proposal = session.begin_proposal(agent_id(), "Wall").unwrap();
        session
            .stage_proposal_ops(
                &proposal.id,
                &agent_id(),
                vec![wall("Proposed wall", [0, 0], [12_000, 0])],
            )
            .unwrap();
        session
            .submit_proposal(&proposal.id, &agent_id(), "Wall")
            .unwrap();

        let base_hash = session.document().document_hash();
        // Create then delete an element: the document hash returns to its base
        // value, but the monotonic id allocator has moved on.
        session
            .apply_batch(player_id(), vec![tower("Temporary drum", [30_000, 0])])
            .unwrap();
        session
            .apply_batch(player_id(), vec![CommandOp::DeleteElement { id: 1 }])
            .unwrap();
        assert_eq!(session.document().document_hash(), base_hash);
        assert_ne!(session.next_element_id(), proposal.base_next_element_id);

        assert_eq!(
            session.accept_proposal(&proposal.id),
            Err(ProposalError::StaleBase)
        );
        assert_eq!(
            session.proposal(&proposal.id).unwrap().status,
            ProposalStatus::Submitted
        );
    }

    #[test]
    fn ghost_is_only_built_for_live_non_empty_proposals() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);

        // An empty draft has nothing to render.
        let empty = session.begin_proposal(agent_id(), "Empty").unwrap();
        assert!(session.build_proposal_ghost_scene(&empty.id).is_none());

        // A live submitted proposal renders.
        let proposal = session.begin_proposal(agent_id(), "Tower").unwrap();
        session
            .stage_proposal_ops(
                &proposal.id,
                &agent_id(),
                vec![tower("Drum tower", [10_000, 0])],
            )
            .unwrap();
        assert!(session.build_proposal_ghost_scene(&proposal.id).is_some());
        session
            .submit_proposal(&proposal.id, &agent_id(), "A drum tower")
            .unwrap();
        assert!(session.build_proposal_ghost_scene(&proposal.id).is_some());

        // Rejected proposals stop rendering a ghost.
        session.reject_proposal(&proposal.id, None).unwrap();
        assert!(session.build_proposal_ghost_scene(&proposal.id).is_none());
    }

    #[test]
    fn accepted_proposals_stop_rendering_a_ghost() {
        let mut session = session_with_agent(AgentPermission::ProposalOnly);
        let proposal = session.begin_proposal(agent_id(), "Tower").unwrap();
        session
            .stage_proposal_ops(
                &proposal.id,
                &agent_id(),
                vec![tower("Drum tower", [10_000, 0])],
            )
            .unwrap();
        session
            .submit_proposal(&proposal.id, &agent_id(), "A drum tower")
            .unwrap();
        session.accept_proposal(&proposal.id).unwrap();
        assert!(session.build_proposal_ghost_scene(&proposal.id).is_none());
    }
}
