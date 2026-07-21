use gareji_board_domain::{
    AgentLoopExecutionTarget, ControlNodeKind, ControlSignal, RouteDecisionReceipt,
    RouteDecisionRequest, WorkItemGraphPosition,
};
use gareji_board_store::{SqliteBoardStore, StoreError};
use thiserror::Error;

/// One declared route available from the current pinned Control node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermittedControlRoute {
    pub route_id: String,
    pub signal: ControlSignal,
    pub next_node_id: String,
}

/// Board-owned interpretation of one Work item's current pinned Control node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CurrentControlNode {
    AgentLoop {
        position: WorkItemGraphPosition,
        target: AgentLoopExecutionTarget,
    },
    AwaitingEvidence {
        position: WorkItemGraphPosition,
        kind: ControlNodeKind,
        permitted_routes: Vec<PermittedControlRoute>,
    },
    AwaitingApproval {
        position: WorkItemGraphPosition,
        permitted_routes: Vec<PermittedControlRoute>,
    },
    Terminal {
        position: WorkItemGraphPosition,
    },
}

/// Explicit evidence-backed request to select one route from a Gate or Audit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceRouteRequest {
    pub decision_id: String,
    pub work_item_id: String,
    pub expected_current_node_id: String,
    pub route_id: String,
    pub evidence_refs: Vec<String>,
}

/// Explicit human decision to select one declared Approval route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HumanApprovalRequest {
    pub decision_id: String,
    pub work_item_id: String,
    pub expected_current_node_id: String,
    pub route_id: String,
    pub evidence_refs: Vec<String>,
}

/// Durable route result and immediate interpretation of its next Control node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlNodeTransitionReceipt {
    pub route: RouteDecisionReceipt,
    pub current: CurrentControlNode,
}

/// Resolves and advances non-Agent Control nodes without starting a Runner.
pub struct ControlNodeController;

impl ControlNodeController {
    /// Resolve one Work item's current pinned Control node.
    ///
    /// # Errors
    ///
    /// Returns a bounded lookup or corrupt-state error.
    pub fn resolve_current_node(
        store: &SqliteBoardStore,
        work_item_id: &str,
    ) -> Result<CurrentControlNode, ControlNodeError> {
        let position = store
            .load_work_item_graph_positions()?
            .into_iter()
            .find(|position| position.work_item_id == work_item_id)
            .ok_or(ControlNodeError::GraphPositionNotFound)?;
        let graph = store
            .load_control_graph_revisions()?
            .into_iter()
            .find(|graph| {
                graph.graph_id == position.graph_id && graph.revision_id == position.revision_id
            })
            .ok_or(ControlNodeError::GraphRevisionNotFound)?;
        let node = graph
            .nodes
            .iter()
            .find(|node| node.id == position.current_node_id)
            .ok_or(ControlNodeError::ControlNodeNotFound)?;
        let permitted_routes = graph
            .routes
            .iter()
            .filter(|route| route.source_node_id == position.current_node_id)
            .map(|route| PermittedControlRoute {
                route_id: route.id.clone(),
                signal: route.signal,
                next_node_id: route.destination_node_id.clone(),
            })
            .collect::<Vec<_>>();
        match &node.kind {
            ControlNodeKind::AgentLoop { .. } => Ok(CurrentControlNode::AgentLoop {
                target: store.load_agent_loop_execution_target(work_item_id)?,
                position,
            }),
            kind @ (ControlNodeKind::Gate | ControlNodeKind::Audit) => {
                Ok(CurrentControlNode::AwaitingEvidence {
                    position,
                    kind: kind.clone(),
                    permitted_routes,
                })
            }
            ControlNodeKind::Approval => Ok(CurrentControlNode::AwaitingApproval {
                position,
                permitted_routes,
            }),
            ControlNodeKind::Terminal => Ok(CurrentControlNode::Terminal { position }),
        }
    }

    /// Record one declared Gate or Audit route with explicit evidence.
    ///
    /// # Errors
    ///
    /// Returns a bounded node-kind, route, evidence, concurrency, or storage error.
    pub fn record_evidence_route(
        store: &mut SqliteBoardStore,
        request: &EvidenceRouteRequest,
    ) -> Result<ControlNodeTransitionReceipt, ControlNodeError> {
        let current = Self::resolve_current_node(store, &request.work_item_id)?;
        let CurrentControlNode::AwaitingEvidence {
            position,
            permitted_routes,
            ..
        } = current
        else {
            return Err(ControlNodeError::EvidenceNotAcceptedAtCurrentNode);
        };
        if position.current_node_id != request.expected_current_node_id {
            return Err(ControlNodeError::CurrentNodeChanged);
        }
        let route = permitted_routes
            .iter()
            .find(|route| route.route_id == request.route_id)
            .ok_or(ControlNodeError::RouteNotPermitted)?;
        let receipt = store.record_route_decision(&RouteDecisionRequest {
            decision_id: request.decision_id.clone(),
            project_id: position.project_id,
            work_item_id: request.work_item_id.clone(),
            expected_current_node_id: request.expected_current_node_id.clone(),
            signal: route.signal,
            proposed_route_id: Some(route.route_id.clone()),
            evidence_refs: request.evidence_refs.clone(),
        })?;
        let current = Self::resolve_current_node(store, &request.work_item_id)?;
        Ok(ControlNodeTransitionReceipt {
            route: receipt,
            current,
        })
    }

    /// Record one explicit human approval using a declared Approval route.
    ///
    /// # Errors
    ///
    /// Returns a bounded node-kind, route, evidence, concurrency, or storage error.
    pub fn record_human_approval(
        store: &mut SqliteBoardStore,
        request: &HumanApprovalRequest,
    ) -> Result<ControlNodeTransitionReceipt, ControlNodeError> {
        let current = Self::resolve_current_node(store, &request.work_item_id)?;
        let CurrentControlNode::AwaitingApproval {
            position,
            permitted_routes,
        } = current
        else {
            return Err(ControlNodeError::ApprovalNotAcceptedAtCurrentNode);
        };
        if position.current_node_id != request.expected_current_node_id {
            return Err(ControlNodeError::CurrentNodeChanged);
        }
        let route = permitted_routes
            .iter()
            .find(|route| route.route_id == request.route_id)
            .ok_or(ControlNodeError::RouteNotPermitted)?;
        if !matches!(
            route.signal,
            ControlSignal::Approved | ControlSignal::Rejected
        ) {
            return Err(ControlNodeError::InvalidApprovalSignal);
        }
        let receipt = store.record_route_decision(&RouteDecisionRequest {
            decision_id: request.decision_id.clone(),
            project_id: position.project_id,
            work_item_id: request.work_item_id.clone(),
            expected_current_node_id: request.expected_current_node_id.clone(),
            signal: route.signal,
            proposed_route_id: Some(route.route_id.clone()),
            evidence_refs: request.evidence_refs.clone(),
        })?;
        let current = Self::resolve_current_node(store, &request.work_item_id)?;
        Ok(ControlNodeTransitionReceipt {
            route: receipt,
            current,
        })
    }
}

#[derive(Debug, Error)]
pub enum ControlNodeError {
    #[error("the Work item does not have a pinned Graph position")]
    GraphPositionNotFound,
    #[error("the pinned Graph revision was not found")]
    GraphRevisionNotFound,
    #[error("the pinned current Control node was not found")]
    ControlNodeNotFound,
    #[error("the current Control node does not accept Gate or Audit evidence")]
    EvidenceNotAcceptedAtCurrentNode,
    #[error("the current Control node changed before evidence was recorded")]
    CurrentNodeChanged,
    #[error("the requested route is not declared from the current Control node")]
    RouteNotPermitted,
    #[error("the current Control node does not accept a human Approval decision")]
    ApprovalNotAcceptedAtCurrentNode,
    #[error("the requested Approval route does not use an approval decision signal")]
    InvalidApprovalSignal,
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[cfg(test)]
mod tests {
    use gareji_board_domain::{
        ControlSignal, ProjectGraphBinding, ProjectGraphBindingSaveRequest, RouteDecisionRequest,
    };

    use super::*;

    #[test]
    fn audit_node_exposes_only_its_declared_evidence_route() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: ProjectGraphBinding {
                    project_id: "gareji-board".to_owned(),
                    graph_id: "direct".to_owned(),
                    revision_id: "v1".to_owned(),
                    entry_id: "standard".to_owned(),
                },
            })
            .unwrap();
        store
            .record_route_decision(&RouteDecisionRequest {
                decision_id: "board-1-implementation-complete".to_owned(),
                project_id: "gareji-board".to_owned(),
                work_item_id: "BOARD-1".to_owned(),
                expected_current_node_id: "implement".to_owned(),
                signal: ControlSignal::Succeeded,
                proposed_route_id: None,
                evidence_refs: vec!["checkpoint:cp-board-1".to_owned()],
            })
            .unwrap();

        let current = ControlNodeController::resolve_current_node(&store, "BOARD-1").unwrap();

        let CurrentControlNode::AwaitingEvidence {
            position,
            kind,
            permitted_routes,
        } = current
        else {
            panic!("verification must wait for independent evidence");
        };
        assert_eq!(position.current_node_id, "verify");
        assert_eq!(kind, ControlNodeKind::Audit);
        assert_eq!(
            permitted_routes,
            vec![PermittedControlRoute {
                route_id: "verification-passed".to_owned(),
                signal: ControlSignal::Passed,
                next_node_id: "finish".to_owned(),
            }]
        );
    }

    #[test]
    fn audit_evidence_advances_only_the_named_declared_route() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: ProjectGraphBinding {
                    project_id: "gareji-board".to_owned(),
                    graph_id: "direct".to_owned(),
                    revision_id: "v1".to_owned(),
                    entry_id: "standard".to_owned(),
                },
            })
            .unwrap();
        store
            .record_route_decision(&RouteDecisionRequest {
                decision_id: "board-1-implementation-complete".to_owned(),
                project_id: "gareji-board".to_owned(),
                work_item_id: "BOARD-1".to_owned(),
                expected_current_node_id: "implement".to_owned(),
                signal: ControlSignal::Succeeded,
                proposed_route_id: None,
                evidence_refs: vec!["checkpoint:cp-board-1".to_owned()],
            })
            .unwrap();

        let receipt = ControlNodeController::record_evidence_route(
            &mut store,
            &EvidenceRouteRequest {
                decision_id: "board-1-verification-passed".to_owned(),
                work_item_id: "BOARD-1".to_owned(),
                expected_current_node_id: "verify".to_owned(),
                route_id: "verification-passed".to_owned(),
                evidence_refs: vec!["test:cargo-test-workspace".to_owned()],
            },
        )
        .unwrap();

        assert_eq!(receipt.route.decision.signal, ControlSignal::Passed);
        assert_eq!(receipt.route.decision.route_id, "verification-passed");
        assert_eq!(
            receipt.route.decision.evidence_refs,
            vec!["test:cargo-test-workspace".to_owned()]
        );
        assert!(matches!(
            receipt.current,
            CurrentControlNode::Terminal { ref position }
                if position.current_node_id == "finish"
        ));
    }

    #[test]
    fn explicit_human_approval_advances_to_the_declared_agent_loop() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: ProjectGraphBinding {
                    project_id: "gareji-board".to_owned(),
                    graph_id: "high-risk".to_owned(),
                    revision_id: "v1".to_owned(),
                    entry_id: "standard".to_owned(),
                },
            })
            .unwrap();
        store
            .record_route_decision(&RouteDecisionRequest {
                decision_id: "board-1-plan-ready".to_owned(),
                project_id: "gareji-board".to_owned(),
                work_item_id: "BOARD-1".to_owned(),
                expected_current_node_id: "plan".to_owned(),
                signal: ControlSignal::NeedsApproval,
                proposed_route_id: None,
                evidence_refs: vec!["checkpoint:cp-board-1-plan".to_owned()],
            })
            .unwrap();

        let receipt = ControlNodeController::record_human_approval(
            &mut store,
            &HumanApprovalRequest {
                decision_id: "board-1-start-approved".to_owned(),
                work_item_id: "BOARD-1".to_owned(),
                expected_current_node_id: "start-approval".to_owned(),
                route_id: "start-approved".to_owned(),
                evidence_refs: vec!["human:desktop-confirmation".to_owned()],
            },
        )
        .unwrap();

        assert_eq!(receipt.route.decision.signal, ControlSignal::Approved);
        assert_eq!(receipt.route.decision.route_id, "start-approved");
        assert!(matches!(
            receipt.current,
            CurrentControlNode::AgentLoop {
                ref position,
                ref target,
            } if position.current_node_id == "implement"
                && target.agent_profile_id == "implementer"
        ));
    }
}
