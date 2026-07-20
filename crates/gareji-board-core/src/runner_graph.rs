use gareji_board_domain::{
    CheckpointOutcome, ControlSignal, RouteDecisionReceipt, RouteDecisionRequest,
};
use gareji_board_runner::{CodexRunDisposition, CodexRunRequest, CodexRunResult, HandoffOutcome};
use gareji_board_store::{SqliteBoardStore, StoreError};
use thiserror::Error;

use crate::RunnerProgressReceipt;

/// Board controller that advances a pinned Control graph from trusted Runner evidence.
pub struct RunnerGraphController<'store> {
    store: &'store mut SqliteBoardStore,
}

impl<'store> RunnerGraphController<'store> {
    #[must_use]
    pub const fn new(store: &'store mut SqliteBoardStore) -> Self {
        Self { store }
    }

    /// Convert one recorded Runner completion into an immutable Route decision.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when Runner and Checkpoint identities differ, the
    /// Run did not complete, graph position is missing, routing is ambiguous, or
    /// Board storage rejects the decision.
    pub fn advance_after_runner(
        &mut self,
        request: &CodexRunRequest,
        result: &CodexRunResult,
        progress: &RunnerProgressReceipt,
    ) -> Result<RunnerRouteOutcome, RunnerGraphError> {
        validate_evidence_identity(request, result, progress)?;
        let route_class = classify_runner_result(result, progress.activity.outcome)?;
        let positions = self.store.load_work_item_graph_positions()?;
        let position = positions
            .iter()
            .find(|position| position.work_item_id == request.work_item.id)
            .ok_or(RunnerGraphError::GraphPositionNotFound)?;
        let target = self
            .store
            .load_agent_loop_execution_target(&request.work_item.id)?;
        if target.agent_profile_id != request.agent_profile.id
            || request.work_item.agent_profile_id.as_deref()
                != Some(target.agent_profile_id.as_str())
        {
            return Err(RunnerGraphError::AgentTargetMismatch);
        }
        if route_class == RunnerRouteClass::Progress {
            return Ok(RunnerRouteOutcome::Stayed {
                current_node_id: position.current_node_id.clone(),
                reason: RunnerRouteStayReason::WorkContinues,
            });
        }
        let graphs = self.store.load_control_graph_revisions()?;
        let graph = graphs
            .iter()
            .find(|graph| {
                graph.graph_id == position.graph_id && graph.revision_id == position.revision_id
            })
            .ok_or(RunnerGraphError::GraphRevisionNotFound)?;
        let signals: &[ControlSignal] = match route_class {
            RunnerRouteClass::Completion => &[
                ControlSignal::Succeeded,
                ControlSignal::Passed,
                ControlSignal::NeedsApproval,
            ],
            RunnerRouteClass::Failure => &[ControlSignal::Failed, ControlSignal::Rejected],
            RunnerRouteClass::Progress => unreachable!("progress returned before route lookup"),
        };
        let matching = graph
            .routes
            .iter()
            .filter(|route| {
                route.source_node_id == position.current_node_id && signals.contains(&route.signal)
            })
            .collect::<Vec<_>>();
        let [route] = matching.as_slice() else {
            if matching.is_empty() {
                let reason = match route_class {
                    RunnerRouteClass::Completion => {
                        RunnerRouteStayReason::NoDeclaredCompletionRoute
                    }
                    RunnerRouteClass::Failure => RunnerRouteStayReason::NoDeclaredFailureRoute,
                    RunnerRouteClass::Progress => unreachable!("progress has no route lookup"),
                };
                return Ok(RunnerRouteOutcome::Stayed {
                    current_node_id: position.current_node_id.clone(),
                    reason,
                });
            }
            return Err(RunnerGraphError::RouteNotDeterministic);
        };
        let receipt = self.store.record_route_decision(&RouteDecisionRequest {
            decision_id: format!("route-{}", request.run_id),
            project_id: request.work_item.project_id.clone(),
            work_item_id: request.work_item.id.clone(),
            expected_current_node_id: position.current_node_id.clone(),
            signal: route.signal,
            proposed_route_id: None,
            evidence_refs: vec![format!("checkpoint:{}", progress.checkpoint_id)],
        })?;
        Ok(RunnerRouteOutcome::Advanced(Box::new(receipt)))
    }
}

/// Bounded graph effect of one recorded Runner result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunnerRouteOutcome {
    Advanced(Box<RouteDecisionReceipt>),
    Stayed {
        current_node_id: String,
        reason: RunnerRouteStayReason,
    },
}

/// Expected reason why trusted Runner evidence leaves the current node unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunnerRouteStayReason {
    WorkContinues,
    NoDeclaredCompletionRoute,
    NoDeclaredFailureRoute,
}

#[derive(Debug, Error)]
pub enum RunnerGraphError {
    #[error("Runner result and Progress Checkpoint identities differ")]
    InvalidRunnerEvidence,
    #[error("Runner work did not complete a Control node")]
    RunDidNotComplete,
    #[error("the Work item does not have a pinned Graph position")]
    GraphPositionNotFound,
    #[error("the pinned Graph revision is unavailable")]
    GraphRevisionNotFound,
    #[error("Runner result does not belong to the current Agent Loop target")]
    AgentTargetMismatch,
    #[error("Runner completion does not map to one deterministic declared route")]
    RouteNotDeterministic,
    #[error("Board route storage rejected the result")]
    Store(#[from] StoreError),
}

fn validate_evidence_identity(
    request: &CodexRunRequest,
    result: &CodexRunResult,
    progress: &RunnerProgressReceipt,
) -> Result<(), RunnerGraphError> {
    if result.run_id != request.run_id
        || progress.activity.checkpoint_id != progress.checkpoint_id
        || progress.activity.project_id != request.work_item.project_id
        || progress.activity.work_item_id.as_deref() != Some(request.work_item.id.as_str())
    {
        return Err(RunnerGraphError::InvalidRunnerEvidence);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunnerRouteClass {
    Progress,
    Completion,
    Failure,
}

fn classify_runner_result(
    result: &CodexRunResult,
    checkpoint_outcome: CheckpointOutcome,
) -> Result<RunnerRouteClass, RunnerGraphError> {
    match result.disposition {
        CodexRunDisposition::Rejected => Err(RunnerGraphError::RunDidNotComplete),
        CodexRunDisposition::Failed | CodexRunDisposition::TimedOut => {
            if checkpoint_outcome == CheckpointOutcome::Failed {
                Ok(RunnerRouteClass::Failure)
            } else {
                Err(RunnerGraphError::InvalidRunnerEvidence)
            }
        }
        CodexRunDisposition::Succeeded => {
            let handoff = result
                .handoff
                .as_ref()
                .ok_or(RunnerGraphError::InvalidRunnerEvidence)?;
            match (handoff.outcome, checkpoint_outcome) {
                (HandoffOutcome::Progress, CheckpointOutcome::Progress) => {
                    Ok(RunnerRouteClass::Progress)
                }
                (HandoffOutcome::Completed, CheckpointOutcome::Completed) => {
                    Ok(RunnerRouteClass::Completion)
                }
                (HandoffOutcome::Blocked, CheckpointOutcome::Blocked)
                | (HandoffOutcome::Failed, CheckpointOutcome::Failed) => {
                    Ok(RunnerRouteClass::Failure)
                }
                _ => Err(RunnerGraphError::InvalidRunnerEvidence),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gareji_board_domain::{
        AgentProfileSummary, ApprovalRequirement, CheckpointOutcome, CheckpointSource,
        ControlSignal, ExecutionWorkspaceConnection, ExecutionWorkspaceKind, ProgressActivity,
        ProjectGraphBinding, ProjectGraphBindingSaveRequest, WorkItemState, WorkItemSummary,
    };
    use gareji_board_runner::{
        CodexHandoff, CodexModelSelection, CodexRunDisposition, CodexRunRequest, CodexRunResult,
        HandoffOutcome, ModelSource, ResolvedModel, WorktreeEvidence,
    };

    #[test]
    fn completed_runner_result_advances_the_declared_route() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: ProjectGraphBinding {
                    project_id: "gareji-board".to_owned(),
                    graph_id: "reviewed".to_owned(),
                    revision_id: "v1".to_owned(),
                    entry_id: "standard".to_owned(),
                },
            })
            .unwrap();
        store
            .prepare_agent_loop_execution_target("gareji-board", "BOARD-1")
            .unwrap();
        let mut controller = RunnerGraphController::new(&mut store);

        let outcome = controller
            .advance_after_runner(&request(), &completed_result(), &progress_receipt())
            .unwrap();

        let RunnerRouteOutcome::Advanced(receipt) = outcome else {
            panic!("completed Runner work should advance the graph");
        };
        assert_eq!(receipt.decision.signal, ControlSignal::Succeeded);
        assert_eq!(receipt.decision.route_id, "research-complete");
        assert_eq!(receipt.resulting_position.current_node_id, "implement");
        assert_eq!(
            receipt.decision.evidence_refs,
            vec!["checkpoint:checkpoint-run-research".to_owned()]
        );
    }

    #[test]
    fn completed_plan_advances_to_its_declared_approval_boundary() {
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
            .prepare_agent_loop_execution_target("gareji-board", "BOARD-1")
            .unwrap();
        let mut controller = RunnerGraphController::new(&mut store);

        let outcome = controller
            .advance_after_runner(&request(), &completed_result(), &progress_receipt())
            .unwrap();

        let RunnerRouteOutcome::Advanced(receipt) = outcome else {
            panic!("a completed plan must reach its declared approval boundary");
        };
        assert_eq!(receipt.decision.signal, ControlSignal::NeedsApproval);
        assert_eq!(receipt.decision.route_id, "plan-ready");
        assert_eq!(receipt.resulting_position.current_node_id, "start-approval");
    }

    #[test]
    fn progress_runner_result_keeps_the_current_node() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: ProjectGraphBinding {
                    project_id: "gareji-board".to_owned(),
                    graph_id: "reviewed".to_owned(),
                    revision_id: "v1".to_owned(),
                    entry_id: "standard".to_owned(),
                },
            })
            .unwrap();
        store
            .prepare_agent_loop_execution_target("gareji-board", "BOARD-1")
            .unwrap();
        let mut result = completed_result();
        result.handoff.as_mut().unwrap().outcome = HandoffOutcome::Progress;
        let mut progress = progress_receipt();
        progress.activity.outcome = CheckpointOutcome::Progress;
        let mut controller = RunnerGraphController::new(&mut store);

        let outcome = controller
            .advance_after_runner(&request(), &result, &progress)
            .unwrap();

        assert_eq!(
            outcome,
            RunnerRouteOutcome::Stayed {
                current_node_id: "research".to_owned(),
                reason: RunnerRouteStayReason::WorkContinues,
            }
        );
        assert!(store.load_route_decisions("BOARD-1").unwrap().is_empty());
        assert_eq!(
            store.load_work_item_graph_positions().unwrap()[0].current_node_id,
            "research"
        );
    }

    #[test]
    fn failed_runner_result_pauses_when_no_failure_route_is_declared() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: ProjectGraphBinding {
                    project_id: "gareji-board".to_owned(),
                    graph_id: "reviewed".to_owned(),
                    revision_id: "v1".to_owned(),
                    entry_id: "standard".to_owned(),
                },
            })
            .unwrap();
        store
            .prepare_agent_loop_execution_target("gareji-board", "BOARD-1")
            .unwrap();
        let mut result = completed_result();
        result.handoff.as_mut().unwrap().outcome = HandoffOutcome::Blocked;
        let mut progress = progress_receipt();
        progress.activity.outcome = CheckpointOutcome::Blocked;
        let mut controller = RunnerGraphController::new(&mut store);

        let outcome = controller
            .advance_after_runner(&request(), &result, &progress)
            .unwrap();

        assert_eq!(
            outcome,
            RunnerRouteOutcome::Stayed {
                current_node_id: "research".to_owned(),
                reason: RunnerRouteStayReason::NoDeclaredFailureRoute,
            }
        );
        assert!(store.load_route_decisions("BOARD-1").unwrap().is_empty());
    }

    #[test]
    fn runner_result_cannot_advance_a_different_agent_loop() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: ProjectGraphBinding {
                    project_id: "gareji-board".to_owned(),
                    graph_id: "reviewed".to_owned(),
                    revision_id: "v1".to_owned(),
                    entry_id: "standard".to_owned(),
                },
            })
            .unwrap();
        store
            .prepare_agent_loop_execution_target("gareji-board", "BOARD-1")
            .unwrap();
        let mut wrong_request = request();
        wrong_request.agent_profile.id = "implementer".to_owned();
        let mut controller = RunnerGraphController::new(&mut store);

        assert!(matches!(
            controller.advance_after_runner(
                &wrong_request,
                &completed_result(),
                &progress_receipt()
            ),
            Err(RunnerGraphError::AgentTargetMismatch)
        ));
        assert!(store.load_route_decisions("BOARD-1").unwrap().is_empty());
    }

    fn request() -> CodexRunRequest {
        CodexRunRequest {
            run_id: "run-research".to_owned(),
            execution_workspace: ExecutionWorkspaceConnection {
                project_id: "gareji-board".to_owned(),
                kind: ExecutionWorkspaceKind::LocalDirectory,
                location: Some("unused-by-controller".to_owned()),
            },
            work_item: WorkItemSummary {
                id: "BOARD-1".to_owned(),
                project_id: "gareji-board".to_owned(),
                title: "Research graph routing".to_owned(),
                priority: 1,
                state: WorkItemState::InProgress,
                approval_requirement: ApprovalRequirement::None,
                dependency_ids: Vec::new(),
                agent_profile_id: Some("researcher".to_owned()),
                required_capabilities: vec!["research".to_owned()],
            },
            agent_profile: AgentProfileSummary {
                id: "researcher".to_owned(),
                role: "Researcher".to_owned(),
                capabilities: vec!["research".to_owned()],
                instruction_ref: None,
                skill_refs: Vec::new(),
            },
            model: CodexModelSelection::default(),
            codex_profile: None,
            timeout_seconds: 1_200,
        }
    }

    fn completed_result() -> CodexRunResult {
        CodexRunResult {
            run_id: "run-research".to_owned(),
            disposition: CodexRunDisposition::Succeeded,
            resolved_model: ResolvedModel {
                model: None,
                source: ModelSource::CodexDefault,
            },
            worktree: Some(WorktreeEvidence {
                root: "run-root".to_owned(),
                working_directory: "run-root/worktree".to_owned(),
                branch: "gareji/run-run-research".to_owned(),
                base_revision: "base".to_owned(),
                final_revision: "final".to_owned(),
                has_uncommitted_changes: true,
                changed_paths: vec!["docs/research.md".to_owned()],
                changed_paths_truncated: false,
                preserved: true,
            }),
            events_path: Some("events".to_owned()),
            handoff_path: Some("handoff".to_owned()),
            handoff: Some(CodexHandoff {
                outcome: HandoffOutcome::Completed,
                summary: "Research completed.".to_owned(),
                verification: vec!["Sources checked".to_owned()],
                risks: Vec::new(),
                next_action: "Implement the result.".to_owned(),
            }),
            failure: None,
        }
    }

    fn progress_receipt() -> RunnerProgressReceipt {
        RunnerProgressReceipt {
            checkpoint_id: "checkpoint-run-research".to_owned(),
            duplicate: false,
            activity: ProgressActivity {
                checkpoint_id: "checkpoint-run-research".to_owned(),
                recorded_at: "2026-07-19T12:00:00Z".to_owned(),
                project_id: "gareji-board".to_owned(),
                work_item_id: Some("BOARD-1".to_owned()),
                source: CheckpointSource::Runner,
                outcome: CheckpointOutcome::Completed,
                summary: "Research completed.".to_owned(),
                recommended_state: Some(WorkItemState::Done),
                deliveries: Vec::new(),
                attachment: None,
                reconciliation: None,
            },
        }
    }
}
