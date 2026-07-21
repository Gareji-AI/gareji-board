//! Board Adapter for the reusable local Core bridge.

mod approach_note;
mod blueprint_application;
mod blueprint_draft;
mod board_run;
mod control_node;
mod definition_editor;
mod graph_draft;
mod graph_rewrite;
mod portfolio_draft;
mod portfolio_orchestration;
mod portfolio_scheduler;
mod runner_graph;
mod runner_progress;

pub use approach_note::{ApproachNoteReadError, MarkdownApproachNoteReader};
pub use blueprint_application::{
    ApproachNotePin, BlueprintApplicationBlock, BlueprintApplicationBlockedReason,
    BlueprintApplicationPlanner, BlueprintApplicationPreview, BlueprintApplicationProposal,
    BlueprintPlanningFacts,
};
pub use blueprint_draft::{BlueprintDraft, BlueprintDraftError};
pub use board_run::{BoardRunController, BoardRunError, BoardRunOutcome, PreparedBoardRun};
pub use control_node::{
    ControlNodeController, ControlNodeError, ControlNodeTransitionReceipt, CurrentControlNode,
    EvidenceRouteRequest, HumanApprovalRequest, PermittedControlRoute,
};
pub use definition_editor::{
    BlueprintEdit, BlueprintEditPlan, ControlGraphEdit, ControlGraphEditPlan, DefinitionEditor,
    DefinitionEditorError,
};
pub use graph_draft::{GraphDraft, GraphDraftError};
pub use graph_rewrite::{GraphRewriteController, GraphRewriteError};
pub use portfolio_draft::{PortfolioDraft, PortfolioDraftError};
pub use portfolio_orchestration::{
    PortfolioOrchestrationController, PortfolioPreviewBlockedReason, PortfolioPreviewFacts,
    PortfolioRunController, PortfolioStepOutcome, PortfolioStepPreview, PortfolioTickError,
    PortfolioTickMode, PortfolioTickReceipt, PortfolioTickRequest,
};
pub use portfolio_scheduler::{
    PortfolioScheduler, PortfolioSchedulerError, PortfolioSchedulerReport, ScheduledPortfolioTick,
};
pub use runner_graph::{
    RunnerGraphController, RunnerGraphError, RunnerRouteOutcome, RunnerRouteStayReason,
};
pub use runner_progress::{
    RunnerCompletionReceipt, RunnerProgressConnector, RunnerProgressError, RunnerProgressReceipt,
};

use std::env;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, FixedOffset};
use gareji_board_domain::{
    ActivityTimeline, CheckpointDelivery, CheckpointDeliveryStatus, CheckpointOutcome,
    CheckpointSource, ProgressActivity, WorkItemState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

const CORE_BRIDGE_PROTOCOL_VERSION: &str = "gareji.core-bridge.v0";
const MAX_BRIDGE_MESSAGE_BYTES: usize = 1_048_576;
const PER_PROJECT_LIMIT: u16 = 6;
const PORTFOLIO_LIMIT: usize = 12;

/// Result of loading Core-owned activity for Board's current portfolio.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActivityLoad {
    pub timeline: ActivityTimeline,
    pub skipped_projects: u32,
}

/// Deep Module that hides Core process lifecycle, framing, validation, and merging.
pub struct CoreProgressReader {
    command: PathBuf,
    database: Option<PathBuf>,
    request_counter: AtomicU64,
}

impl CoreProgressReader {
    /// Resolve the local Core installation from `GAREJI_CORE_BIN` and `GAREJI_CORE_DB`.
    #[must_use]
    pub fn from_environment() -> Self {
        let command = env::var_os("GAREJI_CORE_BIN")
            .map_or_else(|| PathBuf::from("gareji-core"), PathBuf::from);
        let database = env::var_os("GAREJI_CORE_DB").map(PathBuf::from);
        Self::new(command, database)
    }

    /// Construct an explicit local Core Adapter.
    #[must_use]
    pub const fn new(command: PathBuf, database: Option<PathBuf>) -> Self {
        Self {
            command,
            database,
            request_counter: AtomicU64::new(1),
        }
    }

    /// Load and chronologically merge recent activity for the visible projects.
    ///
    /// Projects absent from Core or lacking the progress grant are counted and
    /// skipped without hiding activity from the remaining portfolio.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when Core is unavailable or violates its Interface.
    pub fn load_portfolio_activity(
        &self,
        project_ids: &[String],
    ) -> Result<ActivityLoad, CoreReadError> {
        if project_ids.is_empty() {
            return Ok(ActivityLoad::default());
        }
        let mut process = self.spawn()?;
        let mut timed = Vec::new();
        let mut skipped_projects = 0_u32;
        let mut has_older = false;

        for project_id in project_ids {
            let request_id = self.request_id();
            let request = CoreBridgeRequest {
                protocol_version: CORE_BRIDGE_PROTOCOL_VERSION.to_owned(),
                request_id: request_id.clone(),
                operation: CoreBridgeOperation::ListProgress {
                    project_id: project_id.clone(),
                    work_item_id: None,
                    before_checkpoint_id: None,
                    limit: PER_PROJECT_LIMIT,
                },
            };
            match process.exchange(&request)? {
                CoreBridgeResponse::Ok {
                    protocol_version,
                    request_id: response_id,
                    result,
                } if protocol_version == CORE_BRIDGE_PROTOCOL_VERSION
                    && response_id == request_id =>
                {
                    let decoded = decode_page(result, project_id)?;
                    has_older |= decoded.has_older;
                    timed.extend(decoded.activities);
                }
                CoreBridgeResponse::Error {
                    protocol_version,
                    request_id: response_id,
                    error,
                } if protocol_version == CORE_BRIDGE_PROTOCOL_VERSION
                    && response_id == request_id =>
                {
                    if matches!(
                        error.code,
                        CoreBridgeErrorCode::ProjectNotFound
                            | CoreBridgeErrorCode::PermissionDenied
                    ) {
                        skipped_projects = skipped_projects.saturating_add(1);
                    } else {
                        return Err(CoreReadError::Rejected);
                    }
                }
                _ => return Err(CoreReadError::InvalidResponse),
            }
        }

        Ok(ActivityLoad {
            timeline: merge_activity(timed, has_older),
            skipped_projects,
        })
    }

    fn spawn(&self) -> Result<CoreProcess, CoreReadError> {
        let mut command = Command::new(&self.command);
        if let Some(database) = &self.database {
            command.arg("--database").arg(database);
        }
        let mut child = command
            .arg("bridge")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| CoreReadError::Unavailable)?;
        let stdin = child.stdin.take().ok_or(CoreReadError::Unavailable)?;
        let stdout = child.stdout.take().ok_or(CoreReadError::Unavailable)?;
        Ok(CoreProcess {
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
        })
    }

    fn request_id(&self) -> String {
        let counter = self.request_counter.fetch_add(1, Ordering::Relaxed);
        format!("board-{}-{counter}", std::process::id())
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum CoreReadError {
    #[error("Gareji Core is not available")]
    Unavailable,
    #[error("Gareji Core returned an invalid response")]
    InvalidResponse,
    #[error("Gareji Core rejected the activity request")]
    Rejected,
}

struct CoreProcess {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl CoreProcess {
    fn exchange(
        &mut self,
        request: &CoreBridgeRequest,
    ) -> Result<CoreBridgeResponse, CoreReadError> {
        let payload = serde_json::to_vec(request).map_err(|_| CoreReadError::InvalidResponse)?;
        if payload.len() > MAX_BRIDGE_MESSAGE_BYTES {
            return Err(CoreReadError::InvalidResponse);
        }
        self.stdin
            .write_all(&payload)
            .and_then(|()| self.stdin.write_all(b"\n"))
            .and_then(|()| self.stdin.flush())
            .map_err(|_| CoreReadError::Unavailable)?;
        let mut response = Vec::new();
        if !read_bounded_line(&mut self.stdout, &mut response)? {
            return Err(CoreReadError::Unavailable);
        }
        serde_json::from_slice(&response).map_err(|_| CoreReadError::InvalidResponse)
    }
}

impl Drop for CoreProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    output: &mut Vec<u8>,
) -> Result<bool, CoreReadError> {
    output.clear();
    loop {
        let available = reader.fill_buf().map_err(|_| CoreReadError::Unavailable)?;
        if available.is_empty() {
            return Ok(!output.is_empty());
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |position| position + 1);
        let payload_len = newline.unwrap_or(consumed);
        if output.len().saturating_add(payload_len) > MAX_BRIDGE_MESSAGE_BYTES {
            return Err(CoreReadError::InvalidResponse);
        }
        output.extend_from_slice(&available[..payload_len]);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(true);
        }
    }
}

struct DecodedPage {
    activities: Vec<TimedActivity>,
    has_older: bool,
}

struct TimedActivity {
    recorded_at: DateTime<FixedOffset>,
    activity: ProgressActivity,
}

fn merge_activity(mut timed: Vec<TimedActivity>, mut has_older: bool) -> ActivityTimeline {
    timed.sort_by(|left, right| {
        right.recorded_at.cmp(&left.recorded_at).then_with(|| {
            right
                .activity
                .checkpoint_id
                .cmp(&left.activity.checkpoint_id)
        })
    });
    if timed.len() > PORTFOLIO_LIMIT {
        has_older = true;
        timed.truncate(PORTFOLIO_LIMIT);
    }
    ActivityTimeline {
        activities: timed.into_iter().map(|entry| entry.activity).collect(),
        has_older,
    }
}

fn decode_page(result: Value, expected_project_id: &str) -> Result<DecodedPage, CoreReadError> {
    let page: ListProgressWire =
        serde_json::from_value(result).map_err(|_| CoreReadError::InvalidResponse)?;
    if page.checkpoints.len() > usize::from(PER_PROJECT_LIMIT) {
        return Err(CoreReadError::InvalidResponse);
    }
    let mut activities = Vec::with_capacity(page.checkpoints.len());
    for status in page.checkpoints {
        let checkpoint: CheckpointWire = serde_json::from_value(status.checkpoint)
            .map_err(|_| CoreReadError::InvalidResponse)?;
        if checkpoint.checkpoint_id != status.checkpoint_id
            || checkpoint.project_id != expected_project_id
        {
            return Err(CoreReadError::InvalidResponse);
        }
        let recorded_at = DateTime::parse_from_rfc3339(&checkpoint.recorded_at)
            .map_err(|_| CoreReadError::InvalidResponse)?;
        activities.push(TimedActivity {
            recorded_at,
            activity: ProgressActivity {
                checkpoint_id: checkpoint.checkpoint_id,
                recorded_at: checkpoint.recorded_at,
                project_id: checkpoint.project_id,
                work_item_id: checkpoint.work_item_id,
                source: checkpoint.source,
                outcome: checkpoint.outcome,
                summary: checkpoint.summary,
                recommended_state: checkpoint.recommended_state,
                deliveries: status
                    .deliveries
                    .into_iter()
                    .map(|delivery| CheckpointDelivery {
                        destination_id: delivery.destination_id,
                        status: delivery.status,
                        attempts: delivery.attempts,
                        last_error: delivery.last_error,
                    })
                    .collect(),
                attachment: None,
                reconciliation: None,
            },
        });
    }
    Ok(DecodedPage {
        activities,
        has_older: page.next_cursor.is_some(),
    })
}

#[derive(Debug, Serialize)]
struct CoreBridgeRequest {
    protocol_version: String,
    request_id: String,
    #[serde(flatten)]
    operation: CoreBridgeOperation,
}

#[derive(Debug, Serialize)]
#[serde(tag = "operation", content = "payload", rename_all = "snake_case")]
enum CoreBridgeOperation {
    ListProgress {
        project_id: String,
        work_item_id: Option<String>,
        before_checkpoint_id: Option<String>,
        limit: u16,
    },
    RecordProgress {
        checkpoint: Value,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CoreBridgeResponse {
    Ok {
        protocol_version: String,
        request_id: String,
        result: Value,
    },
    Error {
        protocol_version: String,
        request_id: String,
        error: CoreBridgeError,
    },
}

#[derive(Debug, Deserialize)]
struct CoreBridgeError {
    code: CoreBridgeErrorCode,
    #[allow(dead_code)]
    message: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CoreBridgeErrorCode {
    InvalidRequest,
    PermissionDenied,
    ProjectNotFound,
    WorkItemNotFound,
    WorkItemNotEligible,
    BoardUnavailable,
    WorkspaceNotConnected,
    CheckpointNotFound,
    Conflict,
    InternalError,
}

#[derive(Debug, Deserialize)]
struct ListProgressWire {
    checkpoints: Vec<CheckpointStatusWire>,
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CheckpointStatusWire {
    checkpoint_id: String,
    checkpoint: Value,
    deliveries: Vec<DeliveryWire>,
}

#[derive(Debug, Deserialize)]
struct DeliveryWire {
    destination_id: String,
    status: CheckpointDeliveryStatus,
    attempts: u32,
    last_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CheckpointWire {
    checkpoint_id: String,
    recorded_at: String,
    project_id: String,
    work_item_id: Option<String>,
    source: CheckpointSource,
    outcome: CheckpointOutcome,
    summary: String,
    recommended_state: Option<WorkItemState>,
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::json;

    use super::*;

    fn page(checkpoint_id: &str, recorded_at: &str) -> Value {
        json!({
            "checkpoints": [{
                "checkpoint_id": checkpoint_id,
                "checkpoint": {
                    "checkpoint_id": checkpoint_id,
                    "recorded_at": recorded_at,
                    "project_id": "core",
                    "work_item_id": null,
                    "source": "mcp",
                    "outcome": "progress",
                    "summary": "Progress",
                    "recommended_state": null
                },
                "deliveries": []
            }],
            "next_cursor": null
        })
    }

    #[test]
    fn decodes_a_valid_bounded_activity_page() {
        let page = json!({
            "checkpoints": [{
                "checkpoint_id": "cp-1",
                "checkpoint": {
                    "checkpoint_id": "cp-1",
                    "recorded_at": "2026-07-17T12:00:00+09:00",
                    "project_id": "core",
                    "work_item_id": "CORE-1",
                    "source": "runner",
                    "outcome": "progress",
                    "summary": "Implemented the timeline.",
                    "recommended_state": "in_review"
                },
                "deliveries": [{
                    "destination_id": "notes",
                    "status": "failed",
                    "attempts": 2,
                    "last_error": "workspace unavailable"
                }]
            }],
            "next_cursor": "cp-1"
        });

        let decoded = decode_page(page, "core").unwrap();
        assert!(decoded.has_older);
        assert_eq!(decoded.activities[0].activity.checkpoint_id, "cp-1");
        assert_eq!(
            decoded.activities[0].activity.deliveries[0]
                .last_error
                .as_deref(),
            Some("workspace unavailable")
        );
    }

    #[test]
    fn rejects_checkpoint_identity_or_project_substitution() {
        let page = json!({
            "checkpoints": [{
                "checkpoint_id": "cp-outer",
                "checkpoint": {
                    "checkpoint_id": "cp-inner",
                    "recorded_at": "2026-07-17T12:00:00+09:00",
                    "project_id": "other",
                    "work_item_id": null,
                    "source": "mcp",
                    "outcome": "progress",
                    "summary": "Wrong project",
                    "recommended_state": null
                },
                "deliveries": []
            }],
            "next_cursor": null
        });

        assert!(matches!(
            decode_page(page, "core"),
            Err(CoreReadError::InvalidResponse)
        ));
    }

    #[test]
    fn portfolio_merge_orders_rfc3339_instants_across_offsets() {
        let mut activities = decode_page(page("cp-earlier", "2026-07-17T12:00:00+09:00"), "core")
            .unwrap()
            .activities;
        activities.extend(
            decode_page(page("cp-later", "2026-07-17T03:30:00Z"), "core")
                .unwrap()
                .activities,
        );

        let timeline = merge_activity(activities, false);
        assert_eq!(timeline.activities[0].checkpoint_id, "cp-later");
        assert_eq!(timeline.activities[1].checkpoint_id, "cp-earlier");
    }

    #[test]
    fn response_reader_rejects_oversized_messages() {
        let mut input = Cursor::new(vec![b'x'; MAX_BRIDGE_MESSAGE_BYTES + 1]);
        let mut output = Vec::new();
        assert_eq!(
            read_bounded_line(&mut input, &mut output),
            Err(CoreReadError::InvalidResponse)
        );
    }

    #[test]
    fn missing_core_binary_is_reported_as_unavailable() {
        let reader = CoreProgressReader::new(
            PathBuf::from("gareji-core-binary-that-does-not-exist"),
            None,
        );
        assert_eq!(
            reader.load_portfolio_activity(&["core".to_owned()]),
            Err(CoreReadError::Unavailable)
        );
    }
}
