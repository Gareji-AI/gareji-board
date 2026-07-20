use std::time::{SystemTime, UNIX_EPOCH};

use gareji_board_domain::AgentLoopExecutionTarget;
use gareji_board_runner::{
    CodexModelSelection, CodexRunDisposition, CodexRunRequest, CodexRunResult, CodexRunner,
    RunnerConfigurationError, validate_request,
};
use gareji_board_store::{SqliteBoardStore, StoreError};
use thiserror::Error;

use crate::{
    RunnerCompletionReceipt, RunnerProgressConnector, RunnerProgressError, RunnerRouteOutcome,
};

const DEFAULT_RUN_TIMEOUT_SECONDS: u32 = 1_200;

/// Fully validated, Board-owned plan for one concrete Runner invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedBoardRun {
    pub target: AgentLoopExecutionTarget,
    pub request: CodexRunRequest,
}

/// Complete observable outcome of one attempted current-node execution.
#[derive(Debug)]
pub struct BoardRunOutcome {
    pub prepared: PreparedBoardRun,
    pub result: CodexRunResult,
    pub completion: Result<RunnerCompletionReceipt, RunnerProgressError>,
}

impl BoardRunOutcome {
    /// Render a concise desktop status without exposing private evidence paths.
    #[must_use]
    pub fn status_message(&self) -> String {
        let disposition = match self.result.disposition {
            CodexRunDisposition::Rejected => "was rejected",
            CodexRunDisposition::Succeeded => "succeeded",
            CodexRunDisposition::Failed => "failed",
            CodexRunDisposition::TimedOut => "timed out",
        };
        match &self.completion {
            Ok(completion) => match &completion.route {
                Ok(RunnerRouteOutcome::Advanced(receipt)) => format!(
                    "Run {} {disposition}. Progress was recorded and the Graph advanced to {}.",
                    self.result.run_id, receipt.resulting_position.current_node_id
                ),
                Ok(RunnerRouteOutcome::Stayed {
                    current_node_id, ..
                }) => format!(
                    "Run {} {disposition}. Progress was recorded and the Graph remains at {}.",
                    self.result.run_id, current_node_id
                ),
                Err(error) => format!(
                    "Run {} {disposition}. Progress was recorded, but the Graph was not updated: {error}",
                    self.result.run_id
                ),
            },
            Err(error) => format!(
                "Run {} {disposition}, but its Progress Checkpoint was not recorded: {error}",
                self.result.run_id
            ),
        }
    }
}

/// Deep Module that prepares, executes, records, and routes one current Graph node.
pub struct BoardRunController {
    runner: CodexRunner,
    progress: RunnerProgressConnector,
}

impl BoardRunController {
    /// Resolve the local Runner and Core progress bridge from the environment.
    ///
    /// # Errors
    ///
    /// Returns a bounded configuration failure if local Runner storage cannot be resolved.
    pub fn from_environment() -> Result<Self, BoardRunError> {
        Ok(Self {
            runner: CodexRunner::from_environment()?,
            progress: RunnerProgressConnector::from_environment(),
        })
    }

    /// Resolve and pin the current Graph node, then build one Runner request.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when any Board-owned execution input is missing
    /// or incompatible.
    pub fn prepare_current_agent_loop(
        store: &mut SqliteBoardStore,
        work_item_id: &str,
        run_id: &str,
        model: CodexModelSelection,
    ) -> Result<PreparedBoardRun, BoardRunError> {
        let work_item = store
            .load_work_items()?
            .into_iter()
            .find(|work_item| work_item.id == work_item_id)
            .ok_or(BoardRunError::WorkItemNotFound)?;
        let target =
            store.prepare_agent_loop_execution_target(&work_item.project_id, work_item_id)?;
        let agent_profile = store
            .load_agent_profiles()?
            .into_iter()
            .find(|profile| profile.id == target.agent_profile_id)
            .ok_or(BoardRunError::AgentProfileNotFound)?;
        let declared_capabilities = agent_profile
            .capabilities
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        if work_item
            .required_capabilities
            .iter()
            .any(|required| !declared_capabilities.contains(required.as_str()))
        {
            return Err(BoardRunError::AgentCapabilityMissing);
        }
        let execution_workspace = store
            .load_execution_workspaces()?
            .into_iter()
            .find(|workspace| workspace.project_id == work_item.project_id)
            .filter(|workspace| workspace.location.is_some())
            .ok_or(BoardRunError::WorkspaceNotConnected)?;

        // The durable assignment remains Board scheduling intent. This clone is
        // the concrete current-Graph-stage projection consumed by one Run.
        let mut execution_work_item = work_item;
        execution_work_item.agent_profile_id = Some(target.agent_profile_id.clone());
        let request = CodexRunRequest {
            run_id: run_id.to_owned(),
            execution_workspace,
            work_item: execution_work_item,
            agent_profile,
            model,
            codex_profile: None,
            timeout_seconds: DEFAULT_RUN_TIMEOUT_SECONDS,
        };
        validate_request(&request).map_err(|failure| BoardRunError::InvalidRunnerRequest {
            code: failure.code,
            message: failure.message,
        })?;
        Ok(PreparedBoardRun { target, request })
    }

    /// Execute one current Agent Loop, record its result in Core, and apply the
    /// resulting trusted Checkpoint to the pinned Graph position.
    ///
    /// Runner rejection and Core recording failures remain visible in the
    /// returned outcome; they are not disguised as preparation failures.
    ///
    /// # Errors
    ///
    /// Returns a bounded preparation error before any Runner side effect occurs.
    pub fn run_current_agent_loop(
        &mut self,
        store: &mut SqliteBoardStore,
        work_item_id: &str,
        run_id: &str,
        model: CodexModelSelection,
    ) -> Result<BoardRunOutcome, BoardRunError> {
        let prepared = Self::prepare_current_agent_loop(store, work_item_id, run_id, model)?;
        let result = self.runner.execute(&prepared.request);
        let completion = self
            .progress
            .complete_runner_result(store, &prepared.request, &result);
        Ok(BoardRunOutcome {
            prepared,
            result,
            completion,
        })
    }

    /// Execute one current Agent Loop using a generated stable Run identity and
    /// the normal Codex model precedence.
    ///
    /// # Errors
    ///
    /// Returns a bounded clock or preparation error before Runner execution.
    pub fn run_current_agent_loop_with_defaults(
        &mut self,
        store: &mut SqliteBoardStore,
        work_item_id: &str,
    ) -> Result<BoardRunOutcome, BoardRunError> {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| BoardRunError::SystemClockUnavailable)?;
        let run_id = format!("run-{}-{}", std::process::id(), elapsed.as_millis());
        self.run_current_agent_loop(store, work_item_id, &run_id, CodexModelSelection::default())
    }
}

/// Bounded failure while Board prepares a current Graph node for execution.
#[derive(Debug, Error)]
pub enum BoardRunError {
    #[error("the Work item was not found")]
    WorkItemNotFound,
    #[error("the current Graph node's Agent profile was not found")]
    AgentProfileNotFound,
    #[error("the current Graph node's Agent lacks a required Work item capability")]
    AgentCapabilityMissing,
    #[error("the Work item's project does not have a local Execution workspace")]
    WorkspaceNotConnected,
    #[error("Runner request was rejected by {code}: {message}")]
    InvalidRunnerRequest { code: String, message: String },
    #[error(transparent)]
    RunnerConfiguration(#[from] RunnerConfigurationError),
    #[error("the local system clock could not create a Run identity")]
    SystemClockUnavailable,
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[cfg(test)]
mod tests {
    use gareji_board_domain::{
        ExecutionWorkspaceConnection, ExecutionWorkspaceKind, ExecutionWorkspaceSaveRequest,
        ProjectGraphBinding, ProjectGraphBindingSaveRequest,
    };

    use super::*;

    #[test]
    fn prepares_the_current_graph_agent_and_local_workspace_without_mutating_assignment() {
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
            .save_execution_workspace(&ExecutionWorkspaceSaveRequest {
                expected: Some(ExecutionWorkspaceConnection {
                    project_id: "gareji-board".to_owned(),
                    kind: ExecutionWorkspaceKind::BundledSample,
                    location: None,
                }),
                target: ExecutionWorkspaceConnection {
                    project_id: "gareji-board".to_owned(),
                    kind: ExecutionWorkspaceKind::LocalDirectory,
                    location: Some("C:/work/gareji-board".to_owned()),
                },
            })
            .unwrap();

        let prepared = BoardRunController::prepare_current_agent_loop(
            &mut store,
            "BOARD-3",
            "run-board-3-001",
            CodexModelSelection::default(),
        )
        .unwrap();

        assert_eq!(prepared.target.node_id, "research");
        assert_eq!(prepared.target.agent_profile_id, "researcher");
        assert_eq!(prepared.request.agent_profile.id, "researcher");
        assert_eq!(
            prepared.request.execution_workspace.location.as_deref(),
            Some("C:/work/gareji-board")
        );
        assert_eq!(prepared.request.timeout_seconds, 1_200);
        let stored = store
            .load_work_items()
            .unwrap()
            .into_iter()
            .find(|work_item| work_item.id == "BOARD-3")
            .unwrap();
        assert_eq!(stored.agent_profile_id.as_deref(), Some("researcher"));
    }

    #[test]
    fn refuses_to_prepare_a_run_without_a_connected_local_workspace() {
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

        assert!(matches!(
            BoardRunController::prepare_current_agent_loop(
                &mut store,
                "BOARD-1",
                "run-board-1-001",
                CodexModelSelection::default(),
            ),
            Err(BoardRunError::WorkspaceNotConnected)
        ));
    }

    #[test]
    fn shares_runner_identity_validation_before_execution() {
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
            .save_execution_workspace(&ExecutionWorkspaceSaveRequest {
                expected: Some(ExecutionWorkspaceConnection {
                    project_id: "gareji-board".to_owned(),
                    kind: ExecutionWorkspaceKind::BundledSample,
                    location: None,
                }),
                target: ExecutionWorkspaceConnection {
                    project_id: "gareji-board".to_owned(),
                    kind: ExecutionWorkspaceKind::LocalDirectory,
                    location: Some("C:/work/gareji-board".to_owned()),
                },
            })
            .unwrap();

        let result = BoardRunController::prepare_current_agent_loop(
            &mut store,
            "BOARD-1",
            "Run Has Spaces",
            CodexModelSelection::default(),
        );
        assert!(matches!(
            result,
            Err(BoardRunError::InvalidRunnerRequest { ref code, .. })
                if code == "invalid_identity"
        ));
    }
}
