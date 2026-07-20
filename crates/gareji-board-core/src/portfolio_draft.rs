use gareji_board_domain::{
    PortfolioNode, PortfolioNodeKind, PortfolioOrchestrationRevision, PortfolioPostAction,
    PortfolioProjectSelector, PortfolioRoute, PortfolioSchedule, PortfolioSignal,
};
use thiserror::Error;

/// In-memory editor for one immutable Portfolio orchestration revision.
///
/// The Module permits incomplete topology while editing, but keeps every local
/// contract valid and emits only a fully valid immutable candidate revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioDraft {
    source: PortfolioOrchestrationRevision,
    orchestration: PortfolioOrchestrationRevision,
    history: Vec<PortfolioOrchestrationRevision>,
}

impl PortfolioDraft {
    /// Open one valid immutable revision for editing.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the source revision is invalid.
    pub fn new(source: PortfolioOrchestrationRevision) -> Result<Self, PortfolioDraftError> {
        source
            .validate()
            .map_err(|_| PortfolioDraftError::InvalidOrchestration)?;
        Ok(Self {
            orchestration: source.clone(),
            source,
            history: Vec::new(),
        })
    }

    /// Read the current draft without exposing mutable state.
    #[must_use]
    pub const fn orchestration(&self) -> &PortfolioOrchestrationRevision {
        &self.orchestration
    }

    /// Replace the draft cadence configuration.
    ///
    /// # Errors
    ///
    /// Returns a bounded error for an invalid interval or unchanged value.
    pub fn set_schedule(&mut self, schedule: PortfolioSchedule) -> Result<(), PortfolioDraftError> {
        self.apply_candidate_change(|orchestration| orchestration.schedule = schedule)
    }

    /// Add one runtime Project Selector.
    ///
    /// # Errors
    ///
    /// Returns a bounded error for invalid identity or selector configuration.
    pub fn add_project_selector(
        &mut self,
        node_id: String,
        selector: PortfolioProjectSelector,
    ) -> Result<(), PortfolioDraftError> {
        self.add_node(node_id, PortfolioNodeKind::ProjectSelector { selector })
    }

    /// Add one bounded post-action.
    ///
    /// # Errors
    ///
    /// Returns a bounded error for invalid or duplicate identity.
    pub fn add_post_action(
        &mut self,
        node_id: String,
        action: PortfolioPostAction,
    ) -> Result<(), PortfolioDraftError> {
        self.add_node(node_id, PortfolioNodeKind::PostAction { action })
    }

    /// Add one Terminal node.
    ///
    /// # Errors
    ///
    /// Returns a bounded error for invalid or duplicate identity.
    pub fn add_terminal(&mut self, node_id: String) -> Result<(), PortfolioDraftError> {
        self.add_node(node_id, PortfolioNodeKind::Terminal)
    }

    /// Remove one non-entry node and every incident route.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the node is missing or owns the fixed entry.
    pub fn remove_node(&mut self, node_id: &str) -> Result<(), PortfolioDraftError> {
        if self.orchestration.entry_node_id == node_id {
            return Err(PortfolioDraftError::EntryNodeCannotBeRemoved);
        }
        if !self
            .orchestration
            .nodes
            .iter()
            .any(|node| node.id == node_id)
        {
            return Err(PortfolioDraftError::NodeNotFound);
        }
        self.apply_candidate_change(|orchestration| {
            orchestration.nodes.retain(|node| node.id != node_id);
            orchestration.routes.retain(|route| {
                route.source_node_id != node_id && route.destination_node_id != node_id
            });
        })
    }

    /// Connect two nodes using one bounded Portfolio signal.
    ///
    /// # Errors
    ///
    /// Returns a bounded error for missing nodes, duplicate identity,
    /// ambiguous routing, or invalid source authority.
    pub fn connect(
        &mut self,
        route_id: String,
        source_node_id: String,
        destination_node_id: String,
        signal: PortfolioSignal,
    ) -> Result<(), PortfolioDraftError> {
        self.apply_candidate_change(|orchestration| {
            orchestration.routes.push(PortfolioRoute {
                id: route_id,
                source_node_id,
                destination_node_id,
                signal,
            });
        })
    }

    /// Remove one route.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the route is missing.
    pub fn remove_route(&mut self, route_id: &str) -> Result<(), PortfolioDraftError> {
        if !self
            .orchestration
            .routes
            .iter()
            .any(|route| route.id == route_id)
        {
            return Err(PortfolioDraftError::RouteNotFound);
        }
        self.apply_candidate_change(|orchestration| {
            orchestration.routes.retain(|route| route.id != route_id);
        })
    }

    /// Restore the immediately preceding valid edit state.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when no earlier state exists.
    pub fn undo(&mut self) -> Result<(), PortfolioDraftError> {
        let previous = self
            .history
            .pop()
            .ok_or(PortfolioDraftError::NothingToUndo)?;
        self.orchestration = previous;
        Ok(())
    }

    /// Restore the immutable source while keeping the reset undoable.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the draft is already unchanged.
    pub fn reset(&mut self) -> Result<(), PortfolioDraftError> {
        if !self.has_changes() {
            return Err(PortfolioDraftError::NoChange);
        }
        self.history.push(self.orchestration.clone());
        self.orchestration.clone_from(&self.source);
        Ok(())
    }

    #[must_use]
    pub fn has_changes(&self) -> bool {
        self.orchestration != self.source
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    /// Emit one fully validated immutable candidate revision.
    ///
    /// # Errors
    ///
    /// Returns a bounded error for unchanged, incomplete, or invalid output.
    pub fn revision(
        &self,
        candidate_revision_id: &str,
    ) -> Result<PortfolioOrchestrationRevision, PortfolioDraftError> {
        if !self.has_changes() {
            return Err(PortfolioDraftError::NoChange);
        }
        let mut candidate = self.orchestration.clone();
        candidate_revision_id.clone_into(&mut candidate.revision_id);
        candidate
            .validate()
            .map_err(|_| PortfolioDraftError::IncompleteOrchestration)?;
        Ok(candidate)
    }

    fn add_node(
        &mut self,
        node_id: String,
        kind: PortfolioNodeKind,
    ) -> Result<(), PortfolioDraftError> {
        self.apply_candidate_change(|orchestration| {
            orchestration
                .nodes
                .push(PortfolioNode { id: node_id, kind });
        })
    }

    fn apply_candidate_change(
        &mut self,
        change: impl FnOnce(&mut PortfolioOrchestrationRevision),
    ) -> Result<(), PortfolioDraftError> {
        let mut candidate = self.orchestration.clone();
        change(&mut candidate);
        candidate
            .validate_editable_structure()
            .map_err(|_| PortfolioDraftError::InvalidEdit)?;
        if candidate == self.orchestration {
            return Err(PortfolioDraftError::NoChange);
        }
        self.history.push(self.orchestration.clone());
        self.orchestration = candidate;
        Ok(())
    }
}

/// Bounded Portfolio editor failure suitable for a non-technical UI.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PortfolioDraftError {
    #[error("the Portfolio orchestration revision cannot be opened")]
    InvalidOrchestration,
    #[error("this edit is incompatible with the Portfolio graph contract")]
    InvalidEdit,
    #[error("the scheduled entry node stays fixed")]
    EntryNodeCannotBeRemoved,
    #[error("the selected node was not found")]
    NodeNotFound,
    #[error("the selected route was not found")]
    RouteNotFound,
    #[error("there is no earlier edit to restore")]
    NothingToUndo,
    #[error("the draft has not changed")]
    NoChange,
    #[error("connect every node and keep a Terminal before saving")]
    IncompleteOrchestration,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permits_incomplete_edits_but_emits_only_a_complete_revision() {
        let mut draft = PortfolioDraft::new(sample_revision()).unwrap();

        draft
            .add_post_action("summary".to_owned(), PortfolioPostAction::RecordSummary)
            .unwrap();
        assert_eq!(
            draft.revision("v2"),
            Err(PortfolioDraftError::IncompleteOrchestration)
        );
        draft
            .connect(
                "selector-empty".to_owned(),
                "selector".to_owned(),
                "summary".to_owned(),
                PortfolioSignal::NoCandidate,
            )
            .unwrap();
        draft
            .connect(
                "summary-finish".to_owned(),
                "summary".to_owned(),
                "finish".to_owned(),
                PortfolioSignal::Completed,
            )
            .unwrap();

        let candidate = draft.revision("v2").unwrap();
        assert_eq!(candidate.revision_id, "v2");
        assert_eq!(candidate.nodes.len(), 3);
        assert!(candidate.validate().is_ok());
    }

    #[test]
    fn undo_reset_and_invalid_authority_preserve_valid_edit_state() {
        let mut draft = PortfolioDraft::new(sample_revision()).unwrap();
        let source = draft.orchestration().clone();

        assert_eq!(
            draft.connect(
                "invalid".to_owned(),
                "finish".to_owned(),
                "selector".to_owned(),
                PortfolioSignal::Manual,
            ),
            Err(PortfolioDraftError::InvalidEdit)
        );
        assert_eq!(draft.orchestration(), &source);

        draft
            .set_schedule(PortfolioSchedule::Interval {
                every_minutes: 30,
                enabled: true,
            })
            .unwrap();
        assert!(draft.has_changes());
        draft.undo().unwrap();
        assert_eq!(draft.orchestration(), &source);
        draft.add_terminal("alternate".to_owned()).unwrap();
        draft.reset().unwrap();
        assert_eq!(draft.orchestration(), &source);
        assert!(draft.can_undo());
    }

    fn sample_revision() -> PortfolioOrchestrationRevision {
        PortfolioOrchestrationRevision {
            orchestration_id: "managed-products".to_owned(),
            revision_id: "v1".to_owned(),
            name: "Managed products".to_owned(),
            schedule: PortfolioSchedule::Manual,
            entry_node_id: "selector".to_owned(),
            nodes: vec![
                PortfolioNode {
                    id: "selector".to_owned(),
                    kind: PortfolioNodeKind::ProjectSelector {
                        selector: PortfolioProjectSelector::AllManaged,
                    },
                },
                PortfolioNode {
                    id: "finish".to_owned(),
                    kind: PortfolioNodeKind::Terminal,
                },
            ],
            routes: vec![PortfolioRoute {
                id: "selector-finish".to_owned(),
                source_node_id: "selector".to_owned(),
                destination_node_id: "finish".to_owned(),
                signal: PortfolioSignal::Completed,
            }],
        }
    }
}
