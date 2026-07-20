use gareji_board_domain::{
    ApproachNoteManifest, BlueprintLink, BlueprintLinkKind, BlueprintNode, BlueprintNodeKind,
    OrchestrationBlueprintRevision,
};
use thiserror::Error;

/// In-memory editor for one immutable Orchestration Blueprint revision.
///
/// The Module permits incomplete layout states during editing, but every local
/// contract remains valid and only a completely valid revision can leave it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlueprintDraft {
    source: OrchestrationBlueprintRevision,
    blueprint: OrchestrationBlueprintRevision,
    history: Vec<OrchestrationBlueprintRevision>,
}

impl BlueprintDraft {
    /// Open one valid immutable revision for editing.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the source revision is invalid.
    pub fn new(source: OrchestrationBlueprintRevision) -> Result<Self, BlueprintDraftError> {
        source
            .validate()
            .map_err(|_| BlueprintDraftError::InvalidBlueprint)?;
        Ok(Self {
            blueprint: source.clone(),
            source,
            history: Vec::new(),
        })
    }

    /// Read the current draft without exposing mutable state.
    #[must_use]
    pub const fn blueprint(&self) -> &OrchestrationBlueprintRevision {
        &self.blueprint
    }

    /// Add an Approach node whose typed sockets come only from a validated note.
    ///
    /// # Errors
    ///
    /// Returns a bounded error for an invalid note, identity, or duplicate node.
    pub fn add_approach(
        &mut self,
        node_id: String,
        note: &ApproachNoteManifest,
    ) -> Result<(), BlueprintDraftError> {
        note.validate()
            .map_err(|_| BlueprintDraftError::InvalidApproachNote)?;
        self.add_node(BlueprintNode {
            id: node_id,
            kind: BlueprintNodeKind::Approach {
                approach_id: note.approach_id.clone(),
            },
            inputs: note.inputs.clone(),
            outputs: note.outputs.clone(),
        })
    }

    /// Add one portable stage with its declared typed sockets.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the node is incompatible with this scope.
    pub fn add_node(&mut self, node: BlueprintNode) -> Result<(), BlueprintDraftError> {
        self.apply_candidate_change(|blueprint| blueprint.nodes.push(node))
    }

    /// Remove one non-entry node and all incident links.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the node is missing or owns the fixed entry.
    pub fn remove_node(&mut self, node_id: &str) -> Result<(), BlueprintDraftError> {
        if self.blueprint.entry_node_id == node_id {
            return Err(BlueprintDraftError::EntryNodeCannotBeRemoved);
        }
        if !self.blueprint.nodes.iter().any(|node| node.id == node_id) {
            return Err(BlueprintDraftError::NodeNotFound);
        }
        self.apply_candidate_change(|blueprint| {
            blueprint.nodes.retain(|node| node.id != node_id);
            blueprint.links.retain(|link| {
                link.source_node_id != node_id && link.destination_node_id != node_id
            });
        })
    }

    /// Add one typed data or bounded flow link.
    ///
    /// # Errors
    ///
    /// Returns a bounded error for incompatible sockets, authority, or identity.
    pub fn connect(
        &mut self,
        link_id: String,
        source_node_id: String,
        destination_node_id: String,
        kind: BlueprintLinkKind,
    ) -> Result<(), BlueprintDraftError> {
        self.apply_candidate_change(|blueprint| {
            blueprint.links.push(BlueprintLink {
                id: link_id,
                source_node_id,
                destination_node_id,
                kind,
            });
        })
    }

    /// Remove one declared link.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the link does not exist.
    pub fn remove_link(&mut self, link_id: &str) -> Result<(), BlueprintDraftError> {
        if !self.blueprint.links.iter().any(|link| link.id == link_id) {
            return Err(BlueprintDraftError::LinkNotFound);
        }
        self.apply_candidate_change(|blueprint| {
            blueprint.links.retain(|link| link.id != link_id);
        })
    }

    /// Restore the immediately preceding valid edit state.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when no earlier state exists.
    pub fn undo(&mut self) -> Result<(), BlueprintDraftError> {
        let previous = self
            .history
            .pop()
            .ok_or(BlueprintDraftError::NothingToUndo)?;
        self.blueprint = previous;
        Ok(())
    }

    /// Restore the immutable source while retaining one undo step.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the draft already matches its source.
    pub fn reset(&mut self) -> Result<(), BlueprintDraftError> {
        if !self.has_changes() {
            return Err(BlueprintDraftError::NoChange);
        }
        self.history.push(self.blueprint.clone());
        self.blueprint.clone_from(&self.source);
        Ok(())
    }

    /// Return whether the last edit can be undone.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    /// Return whether the draft differs from its immutable source.
    #[must_use]
    pub fn has_changes(&self) -> bool {
        self.blueprint.nodes != self.source.nodes || self.blueprint.links != self.source.links
    }

    /// Materialize a completely valid immutable candidate revision.
    ///
    /// # Errors
    ///
    /// Returns a bounded error while any node is unreachable or a required stage is absent.
    pub fn revision(
        &self,
        candidate_revision_id: &str,
    ) -> Result<OrchestrationBlueprintRevision, BlueprintDraftError> {
        if !self.has_changes() {
            return Err(BlueprintDraftError::NoChange);
        }
        let mut candidate = self.blueprint.clone();
        candidate_revision_id.clone_into(&mut candidate.revision_id);
        candidate
            .validate()
            .map_err(|_| BlueprintDraftError::IncompleteBlueprint)?;
        Ok(candidate)
    }

    fn apply_candidate_change(
        &mut self,
        change: impl FnOnce(&mut OrchestrationBlueprintRevision),
    ) -> Result<(), BlueprintDraftError> {
        let mut candidate = self.blueprint.clone();
        change(&mut candidate);
        candidate
            .validate_editable_structure()
            .map_err(|_| BlueprintDraftError::InvalidEdit)?;
        if candidate == self.blueprint {
            return Err(BlueprintDraftError::NoChange);
        }
        self.history.push(self.blueprint.clone());
        self.blueprint = candidate;
        Ok(())
    }
}

/// Bounded Blueprint editor failure suitable for a non-technical UI.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum BlueprintDraftError {
    #[error("the Blueprint revision cannot be opened")]
    InvalidBlueprint,
    #[error("the Approach Note cannot be added")]
    InvalidApproachNote,
    #[error("this edit is incompatible with the Blueprint contract")]
    InvalidEdit,
    #[error("the entry node stays fixed")]
    EntryNodeCannotBeRemoved,
    #[error("the selected node was not found")]
    NodeNotFound,
    #[error("the selected connection was not found")]
    LinkNotFound,
    #[error("there is no earlier edit to restore")]
    NothingToUndo,
    #[error("the draft has not changed")]
    NoChange,
    #[error("connect every node and keep the required stages before saving")]
    IncompleteBlueprint,
}

#[cfg(test)]
mod tests {
    use gareji_board_domain::{
        ApproachRisk, BlueprintLink, BlueprintLinkKind, BlueprintNode, BlueprintNodeKind,
        BlueprintScope, ControlSignal, NoteSocketKind, OrchestrationBlueprintRevision,
    };

    use super::{BlueprintDraft, BlueprintDraftError};

    #[test]
    fn permits_incomplete_edits_but_emits_only_a_complete_revision() {
        let mut draft = BlueprintDraft::new(source_blueprint()).unwrap();
        let note = evidence_review_note();

        draft
            .add_approach("review-evidence".to_owned(), &note)
            .unwrap();
        assert_eq!(
            draft.revision("v2"),
            Err(BlueprintDraftError::IncompleteBlueprint)
        );
        draft
            .connect(
                "approach-to-review".to_owned(),
                "approach".to_owned(),
                "review-evidence".to_owned(),
                BlueprintLinkKind::Flow {
                    signal: ControlSignal::Failed,
                },
            )
            .unwrap();
        draft
            .connect(
                "evidence-to-review".to_owned(),
                "approach".to_owned(),
                "review-evidence".to_owned(),
                BlueprintLinkKind::Data {
                    socket: NoteSocketKind::Evidence,
                },
            )
            .unwrap();
        draft
            .connect(
                "review-to-finish".to_owned(),
                "review-evidence".to_owned(),
                "finish".to_owned(),
                BlueprintLinkKind::Flow {
                    signal: ControlSignal::Succeeded,
                },
            )
            .unwrap();

        let candidate = draft.revision("v2").unwrap();
        assert_eq!(candidate.revision_id, "v2");
        assert_eq!(candidate.nodes.len(), 4);
        assert_eq!(candidate.links.len(), 6);
    }

    #[test]
    fn rejects_incompatible_socket_without_changing_the_draft() {
        let mut draft = BlueprintDraft::new(source_blueprint()).unwrap();
        let before = draft.clone();

        assert_eq!(
            draft.connect(
                "invalid-artifact".to_owned(),
                "approach".to_owned(),
                "audit".to_owned(),
                BlueprintLinkKind::Data {
                    socket: NoteSocketKind::Artifact,
                },
            ),
            Err(BlueprintDraftError::InvalidEdit)
        );
        assert_eq!(draft, before);
    }

    #[test]
    fn undo_and_reset_restore_prior_valid_edit_states() {
        let mut draft = BlueprintDraft::new(source_blueprint()).unwrap();
        draft
            .add_approach("review-evidence".to_owned(), &evidence_review_note())
            .unwrap();
        assert!(draft.can_undo());
        draft.undo().unwrap();
        assert!(!draft.has_changes());

        draft
            .add_approach("review-evidence".to_owned(), &evidence_review_note())
            .unwrap();
        draft.reset().unwrap();
        assert!(!draft.has_changes());
        draft.undo().unwrap();
        assert!(draft.has_changes());
    }

    fn source_blueprint() -> OrchestrationBlueprintRevision {
        OrchestrationBlueprintRevision {
            blueprint_id: "evidence-first".to_owned(),
            revision_id: "v1".to_owned(),
            name: "Evidence first".to_owned(),
            scope: BlueprintScope::Project,
            entry_node_id: "approach".to_owned(),
            nodes: vec![
                BlueprintNode {
                    id: "approach".to_owned(),
                    kind: BlueprintNodeKind::Approach {
                        approach_id: "evidence-first".to_owned(),
                    },
                    inputs: vec![NoteSocketKind::Context, NoteSocketKind::WorkItem],
                    outputs: vec![NoteSocketKind::Evidence],
                },
                BlueprintNode {
                    id: "audit".to_owned(),
                    kind: BlueprintNodeKind::Audit,
                    inputs: vec![NoteSocketKind::Evidence],
                    outputs: Vec::new(),
                },
                BlueprintNode {
                    id: "finish".to_owned(),
                    kind: BlueprintNodeKind::Terminal,
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                },
            ],
            links: vec![
                BlueprintLink {
                    id: "approach-succeeded".to_owned(),
                    source_node_id: "approach".to_owned(),
                    destination_node_id: "audit".to_owned(),
                    kind: BlueprintLinkKind::Flow {
                        signal: ControlSignal::Succeeded,
                    },
                },
                BlueprintLink {
                    id: "approach-evidence".to_owned(),
                    source_node_id: "approach".to_owned(),
                    destination_node_id: "audit".to_owned(),
                    kind: BlueprintLinkKind::Data {
                        socket: NoteSocketKind::Evidence,
                    },
                },
                BlueprintLink {
                    id: "audit-passed".to_owned(),
                    source_node_id: "audit".to_owned(),
                    destination_node_id: "finish".to_owned(),
                    kind: BlueprintLinkKind::Flow {
                        signal: ControlSignal::Passed,
                    },
                },
            ],
        }
    }

    fn evidence_review_note() -> gareji_board_domain::ApproachNoteManifest {
        gareji_board_domain::ApproachNoteManifest {
            approach_id: "evidence-review".to_owned(),
            title: "Evidence review".to_owned(),
            absolute_path: std::env::current_dir()
                .unwrap()
                .join("evidence-review.md")
                .to_string_lossy()
                .into_owned(),
            fingerprint: "a".repeat(64),
            required_capabilities: vec!["review".to_owned()],
            inputs: vec![NoteSocketKind::Evidence],
            outputs: vec![NoteSocketKind::Artifact],
            risk: ApproachRisk::Low,
        }
    }
}
