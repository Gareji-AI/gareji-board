use gareji_board_domain::{
    ControlNodeKind, GraphRewriteDecisionReceipt, GraphRewriteDecisionRequest,
    GraphRewriteProposal, GraphRewriteProposalRequest, GraphRewriteProposalStatus,
};
use gareji_board_store::{SqliteBoardStore, StoreError};
use thiserror::Error;

use crate::GraphDraft;

/// Board controller for producing bounded candidate Graph revisions.
pub struct GraphRewriteController;

impl GraphRewriteController {
    /// Clone the selected immutable revision and apply one bounded rewrite.
    ///
    /// The candidate remains outside the normal Graph catalog until a later
    /// explicit approval decision.
    ///
    /// # Errors
    ///
    /// Returns a bounded identity, lookup, node-kind, no-op, validation, or
    /// storage error. Rejected requests do not change the Project binding.
    pub fn propose(
        store: &mut SqliteBoardStore,
        request: &GraphRewriteProposalRequest,
    ) -> Result<GraphRewriteProposal, GraphRewriteError> {
        if request.project_id != request.expected_source_binding.project_id {
            return Err(GraphRewriteError::InvalidRequest);
        }
        let current_binding = store
            .load_project_graph_bindings()?
            .into_iter()
            .find(|binding| binding.project_id == request.project_id)
            .ok_or(GraphRewriteError::ProjectGraphBindingNotFound)?;
        if current_binding != request.expected_source_binding {
            return Err(GraphRewriteError::ConcurrentChange);
        }
        let source_graph = store
            .load_control_graph_revisions()?
            .into_iter()
            .find(|graph| {
                graph.graph_id == current_binding.graph_id
                    && graph.revision_id == current_binding.revision_id
            })
            .ok_or(GraphRewriteError::GraphRevisionNotFound)?;
        let mut draft =
            GraphDraft::new(source_graph).map_err(|_| GraphRewriteError::InvalidRequest)?;
        draft
            .apply_operation(&request.operation)
            .map_err(|_| GraphRewriteError::InvalidRequest)?;
        let candidate_graph = draft
            .revision(&request.candidate_revision_id)
            .map_err(|_| GraphRewriteError::InvalidRequest)?;
        let profiles = store.load_agent_profiles()?;
        if candidate_graph.nodes.iter().any(|node| {
            matches!(
                &node.kind,
                ControlNodeKind::AgentLoop { agent_profile_id }
                    if !profiles.iter().any(|profile| profile.id == *agent_profile_id)
            )
        }) {
            return Err(GraphRewriteError::AgentProfileNotFound);
        }

        let proposal = GraphRewriteProposal {
            proposal_id: request.proposal_id.clone(),
            project_id: request.project_id.clone(),
            source_binding: current_binding,
            candidate_graph,
            operation: request.operation.clone(),
            rationale: request.rationale.clone(),
            evidence_refs: request.evidence_refs.clone(),
            status: GraphRewriteProposalStatus::Pending,
        };
        store
            .save_graph_rewrite_proposal(&proposal)
            .map_err(GraphRewriteError::from)
    }

    /// Apply one explicit human approval or rejection decision.
    ///
    /// # Errors
    ///
    /// Returns a bounded storage error when the proposal is missing, already
    /// decided, conflicts with current Project configuration, or cannot be saved.
    pub fn decide(
        store: &mut SqliteBoardStore,
        request: &GraphRewriteDecisionRequest,
    ) -> Result<GraphRewriteDecisionReceipt, GraphRewriteError> {
        store
            .decide_graph_rewrite_proposal(request)
            .map_err(GraphRewriteError::from)
    }
}

/// Bounded reason why a candidate Graph revision could not be proposed.
#[derive(Debug, Error)]
pub enum GraphRewriteError {
    #[error("Graph rewrite request is invalid")]
    InvalidRequest,
    #[error("Project has no selected Control graph")]
    ProjectGraphBindingNotFound,
    #[error("Project Graph binding changed before the proposal was recorded")]
    ConcurrentChange,
    #[error("selected Control graph revision was not found")]
    GraphRevisionNotFound,
    #[error("replacement Agent profile was not found")]
    AgentProfileNotFound,
    #[error("target Control node was not found")]
    ControlNodeNotFound,
    #[error("target Control node is not an Agent Loop")]
    ControlNodeNotAgentLoop,
    #[error("replacement would not change the Agent profile")]
    NoChange,
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[cfg(test)]
mod tests {
    use gareji_board_domain::{
        ControlNode, ControlNodeKind, ControlRoute, ControlSignal, GraphRewriteDecision,
        GraphRewriteDecisionRequest, GraphRewriteOperation, GraphRewriteProposalRequest,
        GraphRewriteProposalStatus, ProjectGraphBinding, ProjectGraphBindingSaveRequest,
    };
    use gareji_board_store::SqliteBoardStore;

    use super::GraphRewriteController;

    #[test]
    fn pending_agent_replacement_is_recorded_without_changing_the_active_graph() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        let source_binding = ProjectGraphBinding {
            project_id: "gareji-board".to_owned(),
            graph_id: "reviewed".to_owned(),
            revision_id: "v1".to_owned(),
            entry_id: "standard".to_owned(),
        };
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: source_binding.clone(),
            })
            .unwrap();

        let proposal = GraphRewriteController::propose(
            &mut store,
            &GraphRewriteProposalRequest {
                proposal_id: "rewrite-reviewer".to_owned(),
                project_id: "gareji-board".to_owned(),
                expected_source_binding: source_binding.clone(),
                candidate_revision_id: "v2-reviewer".to_owned(),
                operation: GraphRewriteOperation::ReplaceAgentProfile {
                    node_id: "implement".to_owned(),
                    previous_agent_profile_id: "implementer".to_owned(),
                    replacement_agent_profile_id: "reviewer".to_owned(),
                },
                rationale: "Use an independent review profile for the implementation stage."
                    .to_owned(),
                evidence_refs: vec!["checkpoint:cp-42".to_owned()],
            },
        )
        .unwrap();

        assert_eq!(proposal.status, GraphRewriteProposalStatus::Pending);
        assert_eq!(proposal.source_binding, source_binding);
        assert!(matches!(
            proposal
                .candidate_graph
                .nodes
                .iter()
                .find(|node| node.id == "implement")
                .map(|node| &node.kind),
            Some(ControlNodeKind::AgentLoop { agent_profile_id }) if agent_profile_id == "reviewer"
        ));
        assert_eq!(
            store.load_project_graph_bindings().unwrap(),
            vec![source_binding]
        );
        assert!(
            !store
                .load_control_graph_revisions()
                .unwrap()
                .iter()
                .any(|graph| graph.revision_id == "v2-reviewer")
        );
        assert_eq!(
            store
                .load_project_graph_rewrite_proposals("gareji-board")
                .unwrap(),
            vec![proposal]
        );
    }

    #[test]
    fn connected_topology_edit_is_recorded_as_an_unpublished_candidate() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        let source_binding = ProjectGraphBinding {
            project_id: "gareji-board".to_owned(),
            graph_id: "reviewed".to_owned(),
            revision_id: "v1".to_owned(),
            entry_id: "standard".to_owned(),
        };
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: source_binding.clone(),
            })
            .unwrap();

        let proposal = GraphRewriteController::propose(
            &mut store,
            &GraphRewriteProposalRequest {
                proposal_id: "rewrite-security-stage".to_owned(),
                project_id: "gareji-board".to_owned(),
                expected_source_binding: source_binding.clone(),
                candidate_revision_id: "v2-security-stage".to_owned(),
                operation: GraphRewriteOperation::EditTopology {
                    added_nodes: vec![ControlNode {
                        id: "security-review".to_owned(),
                        kind: ControlNodeKind::AgentLoop {
                            agent_profile_id: "reviewer".to_owned(),
                        },
                    }],
                    removed_node_ids: Vec::new(),
                    added_routes: vec![
                        ControlRoute {
                            id: "implementation-security".to_owned(),
                            source_node_id: "implement".to_owned(),
                            destination_node_id: "security-review".to_owned(),
                            signal: ControlSignal::Succeeded,
                        },
                        ControlRoute {
                            id: "security-review-complete".to_owned(),
                            source_node_id: "security-review".to_owned(),
                            destination_node_id: "review".to_owned(),
                            signal: ControlSignal::Succeeded,
                        },
                    ],
                    removed_route_ids: vec!["implementation-complete".to_owned()],
                },
                rationale: "Insert a focused security pass before independent review.".to_owned(),
                evidence_refs: vec!["checkpoint:security-gap".to_owned()],
            },
        )
        .unwrap();

        assert!(
            proposal
                .candidate_graph
                .nodes
                .iter()
                .any(|node| node.id == "security-review")
        );
        assert!(
            !proposal
                .candidate_graph
                .routes
                .iter()
                .any(|route| route.id == "implementation-complete")
        );
        assert_eq!(
            store.load_project_graph_bindings().unwrap(),
            vec![source_binding]
        );
        assert!(
            !store
                .load_control_graph_revisions()
                .unwrap()
                .iter()
                .any(|graph| graph.revision_id == "v2-security-stage")
        );
    }

    #[test]
    fn approval_publishes_the_candidate_for_future_work_without_redirecting_pinned_work() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        let source_binding = ProjectGraphBinding {
            project_id: "gareji-board".to_owned(),
            graph_id: "reviewed".to_owned(),
            revision_id: "v1".to_owned(),
            entry_id: "standard".to_owned(),
        };
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: source_binding.clone(),
            })
            .unwrap();
        store
            .prepare_agent_loop_execution_target("gareji-board", "BOARD-1")
            .unwrap();
        GraphRewriteController::propose(
            &mut store,
            &GraphRewriteProposalRequest {
                proposal_id: "rewrite-reviewer".to_owned(),
                project_id: "gareji-board".to_owned(),
                expected_source_binding: source_binding,
                candidate_revision_id: "v2-reviewer".to_owned(),
                operation: GraphRewriteOperation::ReplaceAgentProfile {
                    node_id: "implement".to_owned(),
                    previous_agent_profile_id: "implementer".to_owned(),
                    replacement_agent_profile_id: "reviewer".to_owned(),
                },
                rationale: "Use an independent review profile for the implementation stage."
                    .to_owned(),
                evidence_refs: vec!["checkpoint:cp-42".to_owned()],
            },
        )
        .unwrap();

        let receipt = GraphRewriteController::decide(
            &mut store,
            &GraphRewriteDecisionRequest {
                proposal_id: "rewrite-reviewer".to_owned(),
                project_id: "gareji-board".to_owned(),
                decision: GraphRewriteDecision::Approve,
            },
        )
        .unwrap();

        assert_eq!(
            receipt.proposal.status,
            GraphRewriteProposalStatus::Approved
        );
        assert!(receipt.published);
        assert_eq!(receipt.resulting_binding.revision_id, "v2-reviewer");
        assert!(
            store
                .load_control_graph_revisions()
                .unwrap()
                .iter()
                .any(|graph| graph.graph_id == "reviewed" && graph.revision_id == "v2-reviewer")
        );
        assert_eq!(
            store.load_project_graph_bindings().unwrap()[0].revision_id,
            "v2-reviewer"
        );
        assert_eq!(
            store.load_work_item_graph_positions().unwrap()[0].revision_id,
            "v1"
        );
    }

    #[test]
    fn rejection_keeps_the_candidate_out_of_the_catalog_and_preserves_the_binding() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        let source_binding = ProjectGraphBinding {
            project_id: "gareji-board".to_owned(),
            graph_id: "reviewed".to_owned(),
            revision_id: "v1".to_owned(),
            entry_id: "standard".to_owned(),
        };
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: source_binding.clone(),
            })
            .unwrap();
        GraphRewriteController::propose(
            &mut store,
            &GraphRewriteProposalRequest {
                proposal_id: "rewrite-rejected".to_owned(),
                project_id: "gareji-board".to_owned(),
                expected_source_binding: source_binding.clone(),
                candidate_revision_id: "v2-rejected".to_owned(),
                operation: GraphRewriteOperation::ReplaceAgentProfile {
                    node_id: "implement".to_owned(),
                    previous_agent_profile_id: "implementer".to_owned(),
                    replacement_agent_profile_id: "reviewer".to_owned(),
                },
                rationale: "Consider a reviewer profile for implementation.".to_owned(),
                evidence_refs: vec!["checkpoint:cp-43".to_owned()],
            },
        )
        .unwrap();

        let receipt = GraphRewriteController::decide(
            &mut store,
            &GraphRewriteDecisionRequest {
                proposal_id: "rewrite-rejected".to_owned(),
                project_id: "gareji-board".to_owned(),
                decision: GraphRewriteDecision::Reject,
            },
        )
        .unwrap();

        assert_eq!(
            receipt.proposal.status,
            GraphRewriteProposalStatus::Rejected
        );
        assert!(!receipt.published);
        assert_eq!(receipt.resulting_binding, source_binding);
        assert!(
            !store
                .load_control_graph_revisions()
                .unwrap()
                .iter()
                .any(|graph| graph.revision_id == "v2-rejected")
        );
    }
}
