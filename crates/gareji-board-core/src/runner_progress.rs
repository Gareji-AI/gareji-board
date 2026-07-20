use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use chrono::{DateTime, SecondsFormat, Utc};
use gareji_board_domain::{
    CheckpointDelivery, CheckpointDeliveryStatus, CheckpointOutcome, CheckpointSource,
    ProgressActivity, WorkItemState,
};
use gareji_board_runner::{
    CodexHandoff, CodexRunDisposition, CodexRunRequest, CodexRunResult, HandoffOutcome,
};
use gareji_board_store::SqliteBoardStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    CORE_BRIDGE_PROTOCOL_VERSION, CoreBridgeErrorCode, CoreBridgeOperation, CoreBridgeRequest,
    CoreBridgeResponse, CoreProgressReader, CoreReadError, RunnerGraphController, RunnerGraphError,
    RunnerRouteOutcome,
};

/// Deep Module that durably records one Runner result and returns its Activity projection.
pub struct RunnerProgressConnector {
    recorder: Box<dyn ProgressPort>,
}

impl RunnerProgressConnector {
    /// Resolve the local Core installation from `GAREJI_CORE_BIN` and `GAREJI_CORE_DB`.
    #[must_use]
    pub fn from_environment() -> Self {
        let command = env::var_os("GAREJI_CORE_BIN")
            .map_or_else(|| PathBuf::from("gareji-core"), PathBuf::from);
        let database = env::var_os("GAREJI_CORE_DB").map(PathBuf::from);
        Self::new(command, database)
    }

    /// Construct an explicit local Core progress Adapter.
    #[must_use]
    pub fn new(command: PathBuf, database: Option<PathBuf>) -> Self {
        Self {
            recorder: Box::new(ProcessProgressPort {
                reader: CoreProgressReader::new(command, database),
                request_counter: AtomicU64::new(1),
            }),
        }
    }

    /// Convert, durably record, and project one completed Runner result.
    ///
    /// Rejected Runs are not recorded because no runtime invocation occurred.
    /// Work item state remains unchanged; `recommended_state` is only visible advice.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the result is inconsistent or Core does not accept it.
    pub fn record_runner_result(
        &mut self,
        request: &CodexRunRequest,
        result: &CodexRunResult,
    ) -> Result<RunnerProgressReceipt, RunnerProgressError> {
        let checkpoint = checkpoint_from_runner(request, result)?;
        let checkpoint_value =
            serde_json::to_value(&checkpoint).map_err(|_| RunnerProgressError::InvalidResult)?;
        let recorded = self.recorder.record(checkpoint_value)?;
        if recorded.checkpoint_id != checkpoint.checkpoint_id {
            return Err(RunnerProgressError::InvalidCoreResponse);
        }
        let deliveries = recorded
            .deliveries
            .into_iter()
            .map(DeliveryWire::into_domain)
            .collect::<Vec<_>>();
        let activity = ProgressActivity {
            checkpoint_id: checkpoint.checkpoint_id.clone(),
            recorded_at: checkpoint.recorded_at,
            project_id: checkpoint.project_id,
            work_item_id: checkpoint.work_item_id,
            source: CheckpointSource::Runner,
            outcome: checkpoint.outcome,
            summary: checkpoint.summary,
            recommended_state: checkpoint.recommended_state,
            deliveries,
            attachment: None,
            reconciliation: None,
        };
        Ok(RunnerProgressReceipt {
            checkpoint_id: checkpoint.checkpoint_id,
            duplicate: recorded.duplicate,
            activity,
        })
    }

    /// Record one Runner result through Core, then apply its trusted Checkpoint
    /// evidence to the Work item's pinned Control graph.
    ///
    /// A graph error is returned inside the receipt so the already durable
    /// Progress Checkpoint is never hidden from the caller.
    ///
    /// # Errors
    ///
    /// Returns a bounded progress error when the Run did not start, its result is
    /// inconsistent, or Core cannot durably accept the Progress Checkpoint.
    pub fn complete_runner_result(
        &mut self,
        store: &mut SqliteBoardStore,
        request: &CodexRunRequest,
        result: &CodexRunResult,
    ) -> Result<RunnerCompletionReceipt, RunnerProgressError> {
        let progress = self.record_runner_result(request, result)?;
        let route =
            RunnerGraphController::new(store).advance_after_runner(request, result, &progress);
        Ok(RunnerCompletionReceipt { progress, route })
    }

    #[cfg(test)]
    fn with_port(recorder: Box<dyn ProgressPort>) -> Self {
        Self { recorder }
    }
}

/// Durable receipt and immediate Board Activity projection for one Runner Checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerProgressReceipt {
    pub checkpoint_id: String,
    pub duplicate: bool,
    pub activity: ProgressActivity,
}

/// Combined durable progress and bounded Graph effect for one Runner completion.
#[derive(Debug)]
pub struct RunnerCompletionReceipt {
    pub progress: RunnerProgressReceipt,
    pub route: Result<RunnerRouteOutcome, RunnerGraphError>,
}

/// Bounded failure before a trustworthy Activity projection can be returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum RunnerProgressError {
    #[error("the Runner did not start, so no Progress Checkpoint was recorded")]
    RunNotStarted,
    #[error("the Runner result is inconsistent or outside the Checkpoint contract")]
    InvalidResult,
    #[error("Gareji Core is unavailable")]
    CoreUnavailable,
    #[error("Gareji Core rejected the Progress Checkpoint")]
    CoreRejected,
    #[error("the Progress Checkpoint conflicts with an existing immutable record")]
    CheckpointConflict,
    #[error("Gareji Core returned an invalid Progress Checkpoint response")]
    InvalidCoreResponse,
}

trait ProgressPort {
    fn record(&mut self, checkpoint: Value) -> Result<RecordReceiptWire, RunnerProgressError>;
}

struct ProcessProgressPort {
    reader: CoreProgressReader,
    request_counter: AtomicU64,
}

impl ProgressPort for ProcessProgressPort {
    fn record(&mut self, checkpoint: Value) -> Result<RecordReceiptWire, RunnerProgressError> {
        let request_id = format!(
            "board-runner-{}-{}",
            std::process::id(),
            self.request_counter.fetch_add(1, Ordering::Relaxed)
        );
        let request = CoreBridgeRequest {
            protocol_version: CORE_BRIDGE_PROTOCOL_VERSION.to_owned(),
            request_id: request_id.clone(),
            operation: CoreBridgeOperation::RecordProgress { checkpoint },
        };
        let mut process = self.reader.spawn().map_err(map_core_read_error)?;
        match process.exchange(&request).map_err(map_core_read_error)? {
            CoreBridgeResponse::Ok {
                protocol_version,
                request_id: response_id,
                result,
            } if protocol_version == CORE_BRIDGE_PROTOCOL_VERSION && response_id == request_id => {
                serde_json::from_value(result).map_err(|_| RunnerProgressError::InvalidCoreResponse)
            }
            CoreBridgeResponse::Error {
                protocol_version,
                request_id: response_id,
                error,
            } if protocol_version == CORE_BRIDGE_PROTOCOL_VERSION && response_id == request_id => {
                if matches!(error.code, CoreBridgeErrorCode::Conflict) {
                    Err(RunnerProgressError::CheckpointConflict)
                } else {
                    Err(RunnerProgressError::CoreRejected)
                }
            }
            _ => Err(RunnerProgressError::InvalidCoreResponse),
        }
    }
}

fn map_core_read_error(error: CoreReadError) -> RunnerProgressError {
    match error {
        CoreReadError::Unavailable => RunnerProgressError::CoreUnavailable,
        CoreReadError::InvalidResponse => RunnerProgressError::InvalidCoreResponse,
        CoreReadError::Rejected => RunnerProgressError::CoreRejected,
    }
}

#[derive(Serialize)]
struct RunnerCheckpoint {
    schema_version: &'static str,
    checkpoint_id: String,
    recorded_at: String,
    project_id: String,
    work_item_id: Option<String>,
    execution_workspace_id: String,
    source: &'static str,
    actor: CheckpointActor,
    outcome: CheckpointOutcome,
    summary: String,
    changed_paths: Vec<String>,
    git: Option<CheckpointGit>,
    verification: Vec<CheckpointVerification>,
    evidence_refs: Vec<String>,
    recommended_state: Option<WorkItemState>,
}

#[derive(Serialize)]
struct CheckpointActor {
    #[serde(rename = "type")]
    kind: &'static str,
    id: String,
}

#[derive(Serialize)]
struct CheckpointGit {
    head: Option<String>,
    branch: Option<String>,
    dirty: bool,
}

#[derive(Serialize)]
struct CheckpointVerification {
    name: String,
    status: &'static str,
    evidence_ref: Option<String>,
}

fn checkpoint_from_runner(
    request: &CodexRunRequest,
    result: &CodexRunResult,
) -> Result<RunnerCheckpoint, RunnerProgressError> {
    if result.run_id != request.run_id {
        return Err(RunnerProgressError::InvalidResult);
    }
    if matches!(result.disposition, CodexRunDisposition::Rejected) {
        return Err(RunnerProgressError::RunNotStarted);
    }
    let (outcome, summary, recommended_state, verification) = checkpoint_semantics(result)?;
    let worktree = result
        .worktree
        .as_ref()
        .ok_or(RunnerProgressError::InvalidResult)?;
    let checkpoint_id = format!("checkpoint-{}", request.run_id);
    let handoff_reference = format!("run://{}/handoff", request.run_id);
    let verification = verification
        .iter()
        .map(|name| CheckpointVerification {
            name: name.clone(),
            status: "unknown",
            evidence_ref: Some(handoff_reference.clone()),
        })
        .collect();
    let mut evidence_refs = vec![
        format!("run://{}/codex-events", request.run_id),
        format!("run://{}/worktree", request.run_id),
    ];
    if result.handoff.is_some() {
        evidence_refs.push(handoff_reference);
    }
    Ok(RunnerCheckpoint {
        schema_version: "gareji.progress-checkpoint.v0",
        checkpoint_id,
        recorded_at: current_timestamp(),
        project_id: request.work_item.project_id.clone(),
        work_item_id: Some(request.work_item.id.clone()),
        execution_workspace_id: request.execution_workspace.project_id.clone(),
        source: "runner",
        actor: CheckpointActor {
            kind: "agent",
            id: request.agent_profile.id.clone(),
        },
        outcome,
        summary,
        changed_paths: worktree.changed_paths.clone(),
        git: Some(CheckpointGit {
            head: Some(worktree.final_revision.clone()),
            branch: Some(worktree.branch.clone()),
            dirty: worktree.has_uncommitted_changes,
        }),
        verification,
        evidence_refs,
        recommended_state,
    })
}

type CheckpointSemantics = (
    CheckpointOutcome,
    String,
    Option<WorkItemState>,
    Vec<String>,
);

fn checkpoint_semantics(
    result: &CodexRunResult,
) -> Result<CheckpointSemantics, RunnerProgressError> {
    if matches!(result.disposition, CodexRunDisposition::Succeeded) {
        let handoff = result
            .handoff
            .as_ref()
            .ok_or(RunnerProgressError::InvalidResult)?;
        return Ok(semantics_from_handoff(handoff));
    }
    let failure = result
        .failure
        .as_ref()
        .ok_or(RunnerProgressError::InvalidResult)?;
    Ok((
        CheckpointOutcome::Failed,
        failure.message.clone(),
        Some(WorkItemState::InReview),
        Vec::new(),
    ))
}

fn semantics_from_handoff(handoff: &CodexHandoff) -> CheckpointSemantics {
    let (outcome, recommended_state) = match handoff.outcome {
        HandoffOutcome::Progress => (CheckpointOutcome::Progress, Some(WorkItemState::InReview)),
        HandoffOutcome::Completed => (CheckpointOutcome::Completed, Some(WorkItemState::Done)),
        HandoffOutcome::Blocked => (CheckpointOutcome::Blocked, Some(WorkItemState::Blocked)),
        HandoffOutcome::Failed => (CheckpointOutcome::Failed, Some(WorkItemState::InReview)),
    };
    (
        outcome,
        handoff.summary.clone(),
        recommended_state,
        handoff.verification.clone(),
    )
}

fn current_timestamp() -> String {
    DateTime::<Utc>::from(SystemTime::now()).to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[derive(Deserialize)]
struct RecordReceiptWire {
    checkpoint_id: String,
    duplicate: bool,
    deliveries: Vec<DeliveryWire>,
}

#[derive(Clone, Debug, Deserialize)]
struct DeliveryWire {
    destination_id: String,
    status: CheckpointDeliveryStatus,
    attempts: u32,
    last_error: Option<String>,
}

impl DeliveryWire {
    fn into_domain(self) -> CheckpointDelivery {
        CheckpointDelivery {
            destination_id: self.destination_id,
            status: self.status,
            attempts: self.attempts,
            last_error: self.last_error,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use gareji_board_domain::{
        AgentProfileSummary, ApprovalRequirement, ExecutionWorkspaceConnection,
        ExecutionWorkspaceKind, ProjectGraphBinding, ProjectGraphBindingSaveRequest,
        WorkItemSummary,
    };
    use gareji_board_runner::{
        CodexModelSelection, CodexRunDisposition, CodexRunResult, HandoffOutcome, ModelSource,
        ResolvedModel, WorktreeEvidence,
    };
    use gareji_board_store::SqliteBoardStore;
    use serde_json::json;

    use super::*;

    #[test]
    fn records_a_runner_checkpoint_and_returns_the_same_activity_projection() {
        let captured = Arc::new(Mutex::new(None));
        let mut connector = RunnerProgressConnector::with_port(Box::new(FakeProgressPort {
            captured: Arc::clone(&captured),
            duplicate: false,
        }));

        let receipt = connector
            .record_runner_result(&request(), &successful_result())
            .unwrap();

        assert_eq!(receipt.checkpoint_id, "checkpoint-run-1");
        assert!(!receipt.duplicate);
        assert_eq!(receipt.activity.source, CheckpointSource::Runner);
        assert_eq!(receipt.activity.outcome, CheckpointOutcome::Completed);
        assert_eq!(
            receipt.activity.recommended_state,
            Some(WorkItemState::Done)
        );
        assert_eq!(receipt.activity.work_item_id.as_deref(), Some("BOARD-1"));
        assert_eq!(receipt.activity.deliveries.len(), 2);

        let checkpoint = captured.lock().unwrap().clone().unwrap();
        let schema: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../schemas/progress-checkpoint-v0.schema.json"
        )))
        .unwrap();
        jsonschema::draft202012::validate(&schema, &checkpoint).unwrap();
        assert_eq!(checkpoint["source"], "runner");
        assert_eq!(checkpoint["actor"]["id"], "implementer");
        assert_eq!(checkpoint["changed_paths"], json!(["src/lib.rs"]));
        assert_eq!(checkpoint["git"]["head"], "final-revision");
        assert_eq!(checkpoint["recommended_state"], "done");
        assert_eq!(checkpoint["evidence_refs"][0], "run://run-1/codex-events");
    }

    #[test]
    fn failed_runner_result_recommends_review_without_changing_work_item_state() {
        let captured = Arc::new(Mutex::new(None));
        let mut connector = RunnerProgressConnector::with_port(Box::new(FakeProgressPort {
            captured: Arc::clone(&captured),
            duplicate: false,
        }));
        let mut result = successful_result();
        result.disposition = CodexRunDisposition::TimedOut;
        result.handoff = None;
        result.failure = Some(gareji_board_runner::RunnerFailure {
            category: gareji_board_runner::RunnerFailureCategory::Timeout,
            code: "codex_timeout".to_owned(),
            message: "Codex exceeded the configured Run deadline.".to_owned(),
        });

        let receipt = connector.record_runner_result(&request(), &result).unwrap();

        assert_eq!(receipt.activity.outcome, CheckpointOutcome::Failed);
        assert_eq!(
            receipt.activity.recommended_state,
            Some(WorkItemState::InReview)
        );
        assert_eq!(request().work_item.state, WorkItemState::Todo);
        assert_eq!(
            captured.lock().unwrap().as_ref().unwrap()["recommended_state"],
            "in_review"
        );
    }

    #[test]
    fn rejected_run_does_not_reach_core() {
        let captured = Arc::new(Mutex::new(None));
        let mut connector = RunnerProgressConnector::with_port(Box::new(FakeProgressPort {
            captured: Arc::clone(&captured),
            duplicate: false,
        }));
        let mut result = successful_result();
        result.disposition = CodexRunDisposition::Rejected;
        result.handoff = None;

        assert_eq!(
            connector.record_runner_result(&request(), &result),
            Err(RunnerProgressError::RunNotStarted)
        );
        assert!(captured.lock().unwrap().is_none());
    }

    #[test]
    fn completion_api_records_progress_and_advances_the_graph() {
        let captured = Arc::new(Mutex::new(None));
        let mut connector = RunnerProgressConnector::with_port(Box::new(FakeProgressPort {
            captured,
            duplicate: false,
        }));
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
        let mut request = request();
        request.execution_workspace.project_id = "gareji-board".to_owned();
        request.work_item.project_id = "gareji-board".to_owned();
        request.work_item.agent_profile_id = Some("researcher".to_owned());
        request.agent_profile.id = "researcher".to_owned();

        let receipt = connector
            .complete_runner_result(&mut store, &request, &successful_result())
            .unwrap();

        assert_eq!(receipt.progress.checkpoint_id, "checkpoint-run-1");
        let Ok(crate::RunnerRouteOutcome::Advanced(route)) = receipt.route else {
            panic!("recorded completion should advance the graph");
        };
        assert_eq!(route.decision.route_id, "research-complete");
        assert_eq!(route.resulting_position.current_node_id, "implement");
    }

    struct FakeProgressPort {
        captured: Arc<Mutex<Option<Value>>>,
        duplicate: bool,
    }

    impl ProgressPort for FakeProgressPort {
        fn record(&mut self, checkpoint: Value) -> Result<RecordReceiptWire, RunnerProgressError> {
            let checkpoint_id = checkpoint["checkpoint_id"].as_str().unwrap().to_owned();
            self.captured.lock().unwrap().replace(checkpoint);
            Ok(RecordReceiptWire {
                checkpoint_id,
                duplicate: self.duplicate,
                deliveries: vec![
                    DeliveryWire {
                        destination_id: "local-json".to_owned(),
                        status: CheckpointDeliveryStatus::Synced,
                        attempts: 1,
                        last_error: None,
                    },
                    DeliveryWire {
                        destination_id: "notes".to_owned(),
                        status: CheckpointDeliveryStatus::Pending,
                        attempts: 0,
                        last_error: None,
                    },
                ],
            })
        }
    }

    fn request() -> CodexRunRequest {
        CodexRunRequest {
            run_id: "run-1".to_owned(),
            execution_workspace: ExecutionWorkspaceConnection {
                project_id: "board".to_owned(),
                kind: ExecutionWorkspaceKind::LocalDirectory,
                location: Some("not-persisted-in-checkpoint".to_owned()),
            },
            work_item: WorkItemSummary {
                id: "BOARD-1".to_owned(),
                project_id: "board".to_owned(),
                title: "Connect Runner progress".to_owned(),
                priority: 1,
                state: WorkItemState::Todo,
                approval_requirement: ApprovalRequirement::None,
                dependency_ids: Vec::new(),
                agent_profile_id: Some("implementer".to_owned()),
                required_capabilities: vec!["implementation".to_owned()],
            },
            agent_profile: AgentProfileSummary {
                id: "implementer".to_owned(),
                role: "Implementer".to_owned(),
                capabilities: vec!["implementation".to_owned()],
                instruction_ref: None,
                skill_refs: Vec::new(),
            },
            model: CodexModelSelection::default(),
            codex_profile: None,
            timeout_seconds: 1_200,
        }
    }

    fn successful_result() -> CodexRunResult {
        CodexRunResult {
            run_id: "run-1".to_owned(),
            disposition: CodexRunDisposition::Succeeded,
            resolved_model: ResolvedModel {
                model: Some("gpt-5.6".to_owned()),
                source: ModelSource::WorkspaceDefault,
            },
            worktree: Some(WorktreeEvidence {
                root: "local-run-root".to_owned(),
                working_directory: "local-run-root/project".to_owned(),
                branch: "gareji/run-run-1".to_owned(),
                base_revision: "base-revision".to_owned(),
                final_revision: "final-revision".to_owned(),
                has_uncommitted_changes: true,
                changed_paths: vec!["src/lib.rs".to_owned()],
                changed_paths_truncated: false,
                preserved: true,
            }),
            events_path: Some("local-events".to_owned()),
            handoff_path: Some("local-handoff".to_owned()),
            handoff: Some(CodexHandoff {
                outcome: HandoffOutcome::Completed,
                summary: "Implemented Runner progress connection.".to_owned(),
                verification: vec!["workspace tests passed".to_owned()],
                risks: Vec::new(),
                next_action: "Review the change.".to_owned(),
            }),
            failure: None,
        }
    }
}
