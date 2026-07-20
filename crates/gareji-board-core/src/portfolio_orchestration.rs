use gareji_board_domain::{
    AgentProfileSummary, ControlGraphRevision, PortfolioApprovalDecision, PortfolioNodeKind,
    PortfolioOrchestrationRevision, PortfolioOrchestrationValidationError, PortfolioPostAction,
    PortfolioProjectSelector, PortfolioRun, PortfolioRunStatus, PortfolioRunStep, PortfolioSignal,
    PortfolioSnapshot, ProjectGraphBinding, SafeAutopilotOutcome, SafeAutopilotPreview,
    WorkItemSummary,
};
use thiserror::Error;

/// Board facts required to preview one portfolio-wide step without mutation.
pub struct PortfolioPreviewFacts<'a> {
    pub portfolio: &'a PortfolioSnapshot,
    pub work_items: &'a [WorkItemSummary],
    pub agent_profiles: &'a [AgentProfileSummary],
    pub project_graph_bindings: &'a [ProjectGraphBinding],
    pub control_graphs: &'a [ControlGraphRevision],
}

/// Pure controller for resolving one bounded scheduled portfolio step.
pub struct PortfolioOrchestrationController;

impl PortfolioOrchestrationController {
    /// Preview the configured entry node and its exact project Graph target.
    ///
    /// The preview never starts a Runner, advances a graph, or records a Run.
    #[must_use]
    pub fn preview_next(
        revision: &PortfolioOrchestrationRevision,
        facts: &PortfolioPreviewFacts<'_>,
    ) -> PortfolioStepPreview {
        let entry = match revision.entry_node() {
            Ok(entry) => entry,
            Err(error) => {
                return PortfolioStepPreview::blocked(
                    revision,
                    &revision.entry_node_id,
                    PortfolioPreviewBlockedReason::InvalidRevision(error),
                );
            }
        };
        Self::preview_node(revision, &entry.id, facts)
    }

    /// Preview one pinned node without advancing or recording a Run.
    #[must_use]
    pub fn preview_node(
        revision: &PortfolioOrchestrationRevision,
        node_id: &str,
        facts: &PortfolioPreviewFacts<'_>,
    ) -> PortfolioStepPreview {
        if let Err(error) = revision.validate() {
            return PortfolioStepPreview::blocked(
                revision,
                node_id,
                PortfolioPreviewBlockedReason::InvalidRevision(error),
            );
        }
        let Some(node) = revision.node(node_id) else {
            return PortfolioStepPreview::blocked(
                revision,
                node_id,
                PortfolioPreviewBlockedReason::NodeNotFound {
                    node_id: node_id.to_owned(),
                },
            );
        };
        let outcome = match &node.kind {
            PortfolioNodeKind::ProjectSelector { selector } => {
                Self::preview_selector(selector, facts)
            }
            PortfolioNodeKind::ProjectInvocation { project_id } => {
                Self::preview_project(project_id, facts)
            }
            PortfolioNodeKind::PostAction { action } => {
                PortfolioStepOutcome::PostAction { action: *action }
            }
            PortfolioNodeKind::Terminal => PortfolioStepOutcome::Terminal,
        };
        PortfolioStepPreview {
            orchestration_id: revision.orchestration_id.clone(),
            revision_id: revision.revision_id.clone(),
            node_id: node.id.clone(),
            outcome,
        }
    }

    fn preview_selector(
        selector: &PortfolioProjectSelector,
        facts: &PortfolioPreviewFacts<'_>,
    ) -> PortfolioStepOutcome {
        let autopilot = match selector {
            PortfolioProjectSelector::AllManaged => SafeAutopilotPreview::evaluate(
                facts.portfolio,
                facts.work_items,
                facts.agent_profiles,
                1,
            ),
            PortfolioProjectSelector::Include { project_ids } => {
                let mut combined_skips = Vec::new();
                let mut final_outcome = None;
                for project_id in project_ids {
                    let preview = SafeAutopilotPreview::evaluate_for_project(
                        facts.portfolio,
                        facts.work_items,
                        facts.agent_profiles,
                        1,
                        project_id,
                    );
                    combined_skips.extend(preview.skipped);
                    match preview.outcome {
                        SafeAutopilotOutcome::Candidate(candidate) => {
                            final_outcome = Some(SafeAutopilotOutcome::Candidate(candidate));
                            break;
                        }
                        SafeAutopilotOutcome::Stop(reason) => {
                            final_outcome = Some(SafeAutopilotOutcome::Stop(reason));
                            break;
                        }
                        SafeAutopilotOutcome::NoCandidate(reason) => {
                            final_outcome = Some(SafeAutopilotOutcome::NoCandidate(reason));
                        }
                    }
                }
                SafeAutopilotPreview {
                    outcome: final_outcome.unwrap_or(SafeAutopilotOutcome::NoCandidate(
                        gareji_board_domain::NoCandidateReason::NoRunnableCandidate,
                    )),
                    skipped: combined_skips,
                }
            }
        };

        let SafeAutopilotOutcome::Candidate(candidate) = &autopilot.outcome else {
            return PortfolioStepOutcome::ProjectSelection {
                project_id: None,
                graph_binding: None,
                autopilot: Box::new(autopilot),
            };
        };
        let project_id = candidate.work_item.project_id.clone();
        match Self::validated_binding(&project_id, facts) {
            Ok(binding) => PortfolioStepOutcome::ProjectSelection {
                project_id: Some(project_id),
                graph_binding: Some(binding),
                autopilot: Box::new(autopilot),
            },
            Err(reason) => PortfolioStepOutcome::Blocked { reason },
        }
    }

    fn preview_project(
        project_id: &str,
        facts: &PortfolioPreviewFacts<'_>,
    ) -> PortfolioStepOutcome {
        let Some(project) = facts
            .portfolio
            .projects
            .iter()
            .find(|project| project.id == project_id)
        else {
            return PortfolioStepOutcome::Blocked {
                reason: PortfolioPreviewBlockedReason::ProjectNotFound {
                    project_id: project_id.to_owned(),
                },
            };
        };
        let binding = match Self::validated_binding(project_id, facts) {
            Ok(binding) => binding,
            Err(reason) => return PortfolioStepOutcome::Blocked { reason },
        };

        let scoped_portfolio = PortfolioSnapshot {
            projects: vec![project.clone()],
        };
        let scoped_work_items = facts
            .work_items
            .iter()
            .filter(|work_item| work_item.project_id == project_id)
            .cloned()
            .collect::<Vec<_>>();
        PortfolioStepOutcome::ProjectInvocation {
            project_id: project_id.to_owned(),
            graph_binding: binding,
            autopilot: Box::new(SafeAutopilotPreview::evaluate(
                &scoped_portfolio,
                &scoped_work_items,
                facts.agent_profiles,
                1,
            )),
        }
    }

    fn validated_binding(
        project_id: &str,
        facts: &PortfolioPreviewFacts<'_>,
    ) -> Result<ProjectGraphBinding, PortfolioPreviewBlockedReason> {
        let Some(binding) = facts
            .project_graph_bindings
            .iter()
            .find(|binding| binding.project_id == project_id)
        else {
            return Err(PortfolioPreviewBlockedReason::ProjectGraphBindingNotFound {
                project_id: project_id.to_owned(),
            });
        };
        let Some(graph) = facts.control_graphs.iter().find(|graph| {
            graph.graph_id == binding.graph_id && graph.revision_id == binding.revision_id
        }) else {
            return Err(PortfolioPreviewBlockedReason::GraphRevisionNotFound {
                graph_id: binding.graph_id.clone(),
                revision_id: binding.revision_id.clone(),
            });
        };
        binding.validate_against(graph).map_err(|_| {
            PortfolioPreviewBlockedReason::InvalidProjectGraphBinding {
                project_id: project_id.to_owned(),
            }
        })?;
        Ok(binding.clone())
    }
}

/// Read-only result of resolving one portfolio orchestration entry node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioStepPreview {
    pub orchestration_id: String,
    pub revision_id: String,
    pub node_id: String,
    pub outcome: PortfolioStepOutcome,
}

impl PortfolioStepPreview {
    fn blocked(
        revision: &PortfolioOrchestrationRevision,
        node_id: &str,
        reason: PortfolioPreviewBlockedReason,
    ) -> Self {
        Self {
            orchestration_id: revision.orchestration_id.clone(),
            revision_id: revision.revision_id.clone(),
            node_id: node_id.to_owned(),
            outcome: PortfolioStepOutcome::Blocked { reason },
        }
    }
}

/// Bounded kind of work selected for the next scheduled tick.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortfolioStepOutcome {
    ProjectSelection {
        project_id: Option<String>,
        graph_binding: Option<ProjectGraphBinding>,
        autopilot: Box<SafeAutopilotPreview>,
    },
    ProjectInvocation {
        project_id: String,
        graph_binding: ProjectGraphBinding,
        autopilot: Box<SafeAutopilotPreview>,
    },
    PostAction {
        action: PortfolioPostAction,
    },
    Terminal,
    Blocked {
        reason: PortfolioPreviewBlockedReason,
    },
}

/// Fail-closed reason why a portfolio step cannot be selected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortfolioPreviewBlockedReason {
    InvalidRevision(PortfolioOrchestrationValidationError),
    NodeNotFound {
        node_id: String,
    },
    ProjectNotFound {
        project_id: String,
    },
    ProjectGraphBindingNotFound {
        project_id: String,
    },
    GraphRevisionNotFound {
        graph_id: String,
        revision_id: String,
    },
    InvalidProjectGraphBinding {
        project_id: String,
    },
}

/// Whether a caller is forcing a human tick or servicing an automatic due time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortfolioTickMode {
    Force,
    DueOnly,
}

/// Small Interface for advancing at most one Portfolio node.
pub struct PortfolioTickRequest<'a> {
    pub revision: &'a PortfolioOrchestrationRevision,
    pub current_run: Option<&'a PortfolioRun>,
    pub run_id: &'a str,
    pub now_epoch_seconds: i64,
    pub mode: PortfolioTickMode,
    pub facts: &'a PortfolioPreviewFacts<'a>,
}

/// Durable state and append-only observation produced by one tick.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioTickReceipt {
    pub previous: Option<PortfolioRun>,
    pub resulting: PortfolioRun,
    pub step: PortfolioRunStep,
    pub preview: PortfolioStepPreview,
}

/// Deep Module that starts, previews, routes, and schedules one bounded tick.
pub struct PortfolioRunController;

impl PortfolioRunController {
    /// Advance exactly one node, never starting a project Runner.
    ///
    /// # Errors
    ///
    /// Returns a fail-closed error for stale, inactive, not-due, or malformed state.
    pub fn tick(
        request: &PortfolioTickRequest<'_>,
    ) -> Result<PortfolioTickReceipt, PortfolioTickError> {
        request
            .revision
            .validate()
            .map_err(PortfolioTickError::InvalidRevision)?;
        let previous = request.current_run.cloned();
        let mut run = previous.clone().unwrap_or_else(|| PortfolioRun {
            run_id: request.run_id.to_owned(),
            orchestration_id: request.revision.orchestration_id.clone(),
            revision_id: request.revision.revision_id.clone(),
            current_node_id: request.revision.entry_node_id.clone(),
            status: PortfolioRunStatus::Active,
            completed_steps: 0,
            next_tick_at_epoch_seconds: request
                .revision
                .schedule
                .interval_seconds()
                .map(|_| request.now_epoch_seconds),
        });
        if run.orchestration_id != request.revision.orchestration_id
            || run.revision_id != request.revision.revision_id
        {
            return Err(PortfolioTickError::RevisionMismatch);
        }
        if run.status != PortfolioRunStatus::Active {
            return Err(PortfolioTickError::RunNotActive(run.status));
        }
        if request.mode == PortfolioTickMode::DueOnly && !run.is_due(request.now_epoch_seconds) {
            return Err(PortfolioTickError::NotDue);
        }

        let preview = PortfolioOrchestrationController::preview_node(
            request.revision,
            &run.current_node_id,
            request.facts,
        );
        let (signal, selected_project_id, terminal, waiting_approval) =
            tick_observation(&preview.outcome);
        let destination_node_id = signal.and_then(|signal| {
            request
                .revision
                .route(&run.current_node_id, signal)
                .map(|route| route.destination_node_id.clone())
        });
        let sequence = run
            .completed_steps
            .checked_add(1)
            .ok_or(PortfolioTickError::StepOverflow)?;
        let step = PortfolioRunStep {
            run_id: run.run_id.clone(),
            sequence,
            node_id: run.current_node_id.clone(),
            signal,
            destination_node_id: destination_node_id.clone(),
            selected_project_id,
            recorded_at_epoch_seconds: request.now_epoch_seconds,
        };
        run.completed_steps = sequence;
        run.status = if terminal {
            PortfolioRunStatus::Completed
        } else if waiting_approval {
            PortfolioRunStatus::WaitingApproval
        } else if let Some(destination) = destination_node_id {
            run.current_node_id = destination;
            PortfolioRunStatus::Active
        } else {
            PortfolioRunStatus::Paused
        };
        run.next_tick_at_epoch_seconds =
            next_tick_at(request.revision, run.status, request.now_epoch_seconds);

        Ok(PortfolioTickReceipt {
            previous,
            resulting: run,
            step,
            preview,
        })
    }

    /// Record one explicit human decision and advance only its declared route.
    ///
    /// # Errors
    ///
    /// Returns a fail-closed error unless the pinned current node is waiting
    /// for a Portfolio approval.
    pub fn resolve_approval(
        revision: &PortfolioOrchestrationRevision,
        current_run: &PortfolioRun,
        decision: PortfolioApprovalDecision,
        now_epoch_seconds: i64,
    ) -> Result<PortfolioTickReceipt, PortfolioTickError> {
        revision
            .validate()
            .map_err(PortfolioTickError::InvalidRevision)?;
        if current_run.orchestration_id != revision.orchestration_id
            || current_run.revision_id != revision.revision_id
        {
            return Err(PortfolioTickError::RevisionMismatch);
        }
        if current_run.status != PortfolioRunStatus::WaitingApproval {
            return Err(PortfolioTickError::NotWaitingApproval);
        }
        let Some(node) = revision.node(&current_run.current_node_id) else {
            return Err(PortfolioTickError::NodeNotFound);
        };
        if !matches!(
            node.kind,
            PortfolioNodeKind::PostAction {
                action: PortfolioPostAction::RequestApproval
            }
        ) {
            return Err(PortfolioTickError::NotApprovalNode);
        }

        let signal = decision.signal();
        let destination_node_id = revision
            .route(&current_run.current_node_id, signal)
            .map(|route| route.destination_node_id.clone());
        let sequence = current_run
            .completed_steps
            .checked_add(1)
            .ok_or(PortfolioTickError::StepOverflow)?;
        let step = PortfolioRunStep {
            run_id: current_run.run_id.clone(),
            sequence,
            node_id: current_run.current_node_id.clone(),
            signal: Some(signal),
            destination_node_id: destination_node_id.clone(),
            selected_project_id: None,
            recorded_at_epoch_seconds: now_epoch_seconds,
        };
        let mut resulting = current_run.clone();
        resulting.completed_steps = sequence;
        resulting.status = if let Some(destination) = destination_node_id {
            resulting.current_node_id = destination;
            PortfolioRunStatus::Active
        } else {
            PortfolioRunStatus::Paused
        };
        resulting.next_tick_at_epoch_seconds =
            next_tick_at(revision, resulting.status, now_epoch_seconds);
        Ok(PortfolioTickReceipt {
            previous: Some(current_run.clone()),
            resulting,
            step,
            preview: PortfolioStepPreview {
                orchestration_id: revision.orchestration_id.clone(),
                revision_id: revision.revision_id.clone(),
                node_id: node.id.clone(),
                outcome: PortfolioStepOutcome::PostAction {
                    action: PortfolioPostAction::RequestApproval,
                },
            },
        })
    }
}

fn next_tick_at(
    revision: &PortfolioOrchestrationRevision,
    status: PortfolioRunStatus,
    now_epoch_seconds: i64,
) -> Option<i64> {
    matches!(
        status,
        PortfolioRunStatus::Active | PortfolioRunStatus::Completed
    )
    .then(|| revision.schedule.interval_seconds())
    .flatten()
    .map(|interval| now_epoch_seconds.saturating_add(interval))
}

fn tick_observation(
    outcome: &PortfolioStepOutcome,
) -> (Option<PortfolioSignal>, Option<String>, bool, bool) {
    match outcome {
        PortfolioStepOutcome::ProjectSelection {
            project_id,
            autopilot,
            ..
        } => match &autopilot.outcome {
            SafeAutopilotOutcome::Candidate(_) => (
                Some(PortfolioSignal::Completed),
                project_id.clone(),
                false,
                false,
            ),
            SafeAutopilotOutcome::NoCandidate(_) => {
                (Some(PortfolioSignal::NoCandidate), None, false, false)
            }
            SafeAutopilotOutcome::Stop(_) => {
                (Some(PortfolioSignal::NeedsAttention), None, false, false)
            }
        },
        PortfolioStepOutcome::ProjectInvocation {
            project_id,
            autopilot,
            ..
        } => match &autopilot.outcome {
            SafeAutopilotOutcome::Candidate(_) => (
                Some(PortfolioSignal::Completed),
                Some(project_id.clone()),
                false,
                false,
            ),
            SafeAutopilotOutcome::NoCandidate(_) => {
                (Some(PortfolioSignal::NoCandidate), None, false, false)
            }
            SafeAutopilotOutcome::Stop(_) => {
                (Some(PortfolioSignal::NeedsAttention), None, false, false)
            }
        },
        PortfolioStepOutcome::PostAction {
            action: PortfolioPostAction::RecordSummary,
        } => (Some(PortfolioSignal::Completed), None, false, false),
        PortfolioStepOutcome::PostAction {
            action: PortfolioPostAction::RequestApproval,
        } => (None, None, false, true),
        PortfolioStepOutcome::Terminal => (None, None, true, false),
        PortfolioStepOutcome::Blocked { .. } => {
            (Some(PortfolioSignal::NeedsAttention), None, false, false)
        }
    }
}

/// Fail-closed reason why a Portfolio tick was not recorded.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PortfolioTickError {
    #[error("invalid Portfolio orchestration revision: {0}")]
    InvalidRevision(PortfolioOrchestrationValidationError),
    #[error("Portfolio Run is pinned to another revision")]
    RevisionMismatch,
    #[error("Portfolio Run is not active: {0:?}")]
    RunNotActive(PortfolioRunStatus),
    #[error("Portfolio Run is not waiting for approval")]
    NotWaitingApproval,
    #[error("Portfolio Run current node is unavailable")]
    NodeNotFound,
    #[error("Portfolio Run current node is not an approval node")]
    NotApprovalNode,
    #[error("Portfolio Run is not due")]
    NotDue,
    #[error("Portfolio Run step counter overflowed")]
    StepOverflow,
}

#[cfg(test)]
mod tests {
    use gareji_board_domain::{
        AgentProfileSummary, ApprovalRequirement, ControlGraphRevision, ControlNode,
        ControlNodeKind, GraphAnchor, GraphEntry, PortfolioApprovalDecision, PortfolioNode,
        PortfolioNodeKind, PortfolioOrchestrationRevision, PortfolioPostAction,
        PortfolioProjectSelector, PortfolioRoute, PortfolioRunStatus, PortfolioSchedule,
        PortfolioSignal, PortfolioSnapshot, ProjectGraphBinding, ProjectHealth, ProjectSummary,
        WorkItemCounts, WorkItemState, WorkItemSummary,
    };

    use super::{
        PortfolioOrchestrationController, PortfolioPreviewBlockedReason, PortfolioPreviewFacts,
        PortfolioRunController, PortfolioStepOutcome, PortfolioTickMode, PortfolioTickRequest,
    };

    #[test]
    fn resolves_one_project_scoped_autopilot_preview() {
        let revision = orchestration("gareji-board");
        let binding = binding("gareji-board");
        let graph = project_graph();
        let portfolio = PortfolioSnapshot {
            projects: vec![project("gareji-board"), project("gareji-core")],
        };
        let work_items = vec![work_item("BOARD-1", "gareji-board")];
        let profiles = vec![AgentProfileSummary {
            id: "implementer".to_owned(),
            role: "Implementer".to_owned(),
            capabilities: vec!["implementation".to_owned()],
            instruction_ref: None,
            skill_refs: Vec::new(),
        }];

        let preview = PortfolioOrchestrationController::preview_next(
            &revision,
            &PortfolioPreviewFacts {
                portfolio: &portfolio,
                work_items: &work_items,
                agent_profiles: &profiles,
                project_graph_bindings: &[binding],
                control_graphs: &[graph],
            },
        );

        let PortfolioStepOutcome::ProjectInvocation {
            project_id,
            graph_binding,
            autopilot,
        } = preview.outcome
        else {
            panic!("expected project invocation");
        };
        assert_eq!(project_id, "gareji-board");
        assert_eq!(graph_binding.entry_id, "standard");
        assert!(matches!(
            autopilot.outcome,
            gareji_board_domain::SafeAutopilotOutcome::Candidate(_)
        ));
    }

    #[test]
    fn fails_closed_when_the_project_has_no_control_graph() {
        let revision = orchestration("gareji-board");
        let portfolio = PortfolioSnapshot {
            projects: vec![project("gareji-board")],
        };
        let preview = PortfolioOrchestrationController::preview_next(
            &revision,
            &PortfolioPreviewFacts {
                portfolio: &portfolio,
                work_items: &[],
                agent_profiles: &[],
                project_graph_bindings: &[],
                control_graphs: &[],
            },
        );

        assert_eq!(
            preview.outcome,
            PortfolioStepOutcome::Blocked {
                reason: PortfolioPreviewBlockedReason::ProjectGraphBindingNotFound {
                    project_id: "gareji-board".to_owned(),
                }
            }
        );
    }

    #[test]
    fn one_tick_selects_across_projects_and_advances_exactly_one_node() {
        let revision = selector_orchestration();
        let portfolio = PortfolioSnapshot {
            projects: vec![project("gareji-board"), project("gareji-core")],
        };
        let work_items = vec![work_item("BOARD-1", "gareji-board")];
        let profiles = vec![AgentProfileSummary {
            id: "implementer".to_owned(),
            role: "Implementer".to_owned(),
            capabilities: vec!["implementation".to_owned()],
            instruction_ref: None,
            skill_refs: Vec::new(),
        }];
        let bindings = [binding("gareji-board")];
        let graphs = [project_graph()];
        let facts = PortfolioPreviewFacts {
            portfolio: &portfolio,
            work_items: &work_items,
            agent_profiles: &profiles,
            project_graph_bindings: &bindings,
            control_graphs: &graphs,
        };

        let first = PortfolioRunController::tick(&PortfolioTickRequest {
            revision: &revision,
            current_run: None,
            run_id: "run-1",
            now_epoch_seconds: 1_000,
            mode: PortfolioTickMode::Force,
            facts: &facts,
        })
        .unwrap();

        assert_eq!(first.step.node_id, "select-project");
        assert_eq!(
            first.step.selected_project_id.as_deref(),
            Some("gareji-board")
        );
        assert_eq!(first.resulting.current_node_id, "finish");
        assert_eq!(first.resulting.status, PortfolioRunStatus::Active);
        assert_eq!(first.resulting.completed_steps, 1);

        let second = PortfolioRunController::tick(&PortfolioTickRequest {
            revision: &revision,
            current_run: Some(&first.resulting),
            run_id: "unused",
            now_epoch_seconds: 1_001,
            mode: PortfolioTickMode::Force,
            facts: &facts,
        })
        .unwrap();
        assert_eq!(second.step.node_id, "finish");
        assert_eq!(second.resulting.status, PortfolioRunStatus::Completed);
        assert_eq!(second.resulting.completed_steps, 2);
    }

    #[test]
    fn approval_resolution_records_the_human_signal_and_declared_route() {
        let revision = approval_orchestration();
        let portfolio = PortfolioSnapshot::default();
        let facts = PortfolioPreviewFacts {
            portfolio: &portfolio,
            work_items: &[],
            agent_profiles: &[],
            project_graph_bindings: &[],
            control_graphs: &[],
        };
        let waiting = PortfolioRunController::tick(&PortfolioTickRequest {
            revision: &revision,
            current_run: None,
            run_id: "approval-run",
            now_epoch_seconds: 10,
            mode: PortfolioTickMode::Force,
            facts: &facts,
        })
        .unwrap();
        assert_eq!(
            waiting.resulting.status,
            PortfolioRunStatus::WaitingApproval
        );
        assert_eq!(waiting.resulting.current_node_id, "approval");

        let approved = PortfolioRunController::resolve_approval(
            &revision,
            &waiting.resulting,
            PortfolioApprovalDecision::Approved,
            11,
        )
        .unwrap();

        assert_eq!(approved.step.signal, Some(PortfolioSignal::Approved));
        assert_eq!(
            approved.step.destination_node_id.as_deref(),
            Some("accepted")
        );
        assert_eq!(approved.resulting.current_node_id, "accepted");
        assert_eq!(approved.resulting.status, PortfolioRunStatus::Active);
        assert_eq!(approved.resulting.completed_steps, 2);
        assert!(matches!(
            PortfolioRunController::resolve_approval(
                &revision,
                &approved.resulting,
                PortfolioApprovalDecision::Rejected,
                12,
            ),
            Err(super::PortfolioTickError::NotWaitingApproval)
        ));
    }

    fn orchestration(project_id: &str) -> PortfolioOrchestrationRevision {
        PortfolioOrchestrationRevision {
            orchestration_id: "daily-portfolio".to_owned(),
            revision_id: "v1".to_owned(),
            name: "Daily portfolio".to_owned(),
            schedule: PortfolioSchedule::Manual,
            entry_node_id: "project".to_owned(),
            nodes: vec![
                PortfolioNode {
                    id: "project".to_owned(),
                    kind: PortfolioNodeKind::ProjectInvocation {
                        project_id: project_id.to_owned(),
                    },
                },
                PortfolioNode {
                    id: "finish".to_owned(),
                    kind: PortfolioNodeKind::Terminal,
                },
            ],
            routes: vec![PortfolioRoute {
                id: "project-completed".to_owned(),
                source_node_id: "project".to_owned(),
                destination_node_id: "finish".to_owned(),
                signal: PortfolioSignal::Completed,
            }],
        }
    }

    fn selector_orchestration() -> PortfolioOrchestrationRevision {
        PortfolioOrchestrationRevision {
            orchestration_id: "managed-products".to_owned(),
            revision_id: "v2".to_owned(),
            name: "Managed products".to_owned(),
            schedule: PortfolioSchedule::Interval {
                every_minutes: 60,
                enabled: true,
            },
            entry_node_id: "select-project".to_owned(),
            nodes: vec![
                PortfolioNode {
                    id: "select-project".to_owned(),
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
                id: "selected".to_owned(),
                source_node_id: "select-project".to_owned(),
                destination_node_id: "finish".to_owned(),
                signal: PortfolioSignal::Completed,
            }],
        }
    }

    fn approval_orchestration() -> PortfolioOrchestrationRevision {
        PortfolioOrchestrationRevision {
            orchestration_id: "approval-flow".to_owned(),
            revision_id: "v1".to_owned(),
            name: "Approval flow".to_owned(),
            schedule: PortfolioSchedule::Manual,
            entry_node_id: "approval".to_owned(),
            nodes: vec![
                PortfolioNode {
                    id: "approval".to_owned(),
                    kind: PortfolioNodeKind::PostAction {
                        action: PortfolioPostAction::RequestApproval,
                    },
                },
                PortfolioNode {
                    id: "accepted".to_owned(),
                    kind: PortfolioNodeKind::Terminal,
                },
                PortfolioNode {
                    id: "rejected".to_owned(),
                    kind: PortfolioNodeKind::Terminal,
                },
            ],
            routes: vec![
                PortfolioRoute {
                    id: "approve".to_owned(),
                    source_node_id: "approval".to_owned(),
                    destination_node_id: "accepted".to_owned(),
                    signal: PortfolioSignal::Approved,
                },
                PortfolioRoute {
                    id: "reject".to_owned(),
                    source_node_id: "approval".to_owned(),
                    destination_node_id: "rejected".to_owned(),
                    signal: PortfolioSignal::Rejected,
                },
            ],
        }
    }

    fn binding(project_id: &str) -> ProjectGraphBinding {
        ProjectGraphBinding {
            project_id: project_id.to_owned(),
            graph_id: "direct".to_owned(),
            revision_id: "v1".to_owned(),
            entry_id: "standard".to_owned(),
        }
    }

    fn project_graph() -> ControlGraphRevision {
        ControlGraphRevision {
            graph_id: "direct".to_owned(),
            revision_id: "v1".to_owned(),
            entries: vec![GraphEntry {
                id: "standard".to_owned(),
                node_id: "implement".to_owned(),
            }],
            nodes: vec![ControlNode {
                id: "implement".to_owned(),
                kind: ControlNodeKind::AgentLoop {
                    agent_profile_id: "implementer".to_owned(),
                },
            }],
            routes: Vec::new(),
            anchors: vec![GraphAnchor {
                id: "bounded".to_owned(),
                description: "One bounded project step".to_owned(),
            }],
        }
    }

    fn project(id: &str) -> ProjectSummary {
        ProjectSummary {
            id: id.to_owned(),
            name: id.to_owned(),
            health: ProjectHealth::Healthy,
            execution_cap: 1,
            work_items: WorkItemCounts::default(),
        }
    }

    fn work_item(id: &str, project_id: &str) -> WorkItemSummary {
        WorkItemSummary {
            id: id.to_owned(),
            project_id: project_id.to_owned(),
            title: id.to_owned(),
            priority: 1,
            state: WorkItemState::Todo,
            approval_requirement: ApprovalRequirement::None,
            dependency_ids: Vec::new(),
            agent_profile_id: Some("implementer".to_owned()),
            required_capabilities: vec!["implementation".to_owned()],
        }
    }
}
