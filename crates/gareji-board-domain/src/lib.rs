//! Board-owned portfolio terminology and read models.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Stable Work item lifecycle owned by Gareji Board.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemState {
    Backlog,
    Todo,
    InProgress,
    InReview,
    Blocked,
    Done,
    Cancelled,
}

impl WorkItemState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::InReview => "in_review",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }

    /// Assess whether current direct or Runner work may reference this item.
    #[must_use]
    pub const fn active_work_eligibility(self) -> ActiveWorkEligibility {
        match self {
            Self::Todo | Self::InProgress | Self::InReview => ActiveWorkEligibility::Eligible,
            Self::Backlog => ActiveWorkEligibility::Ineligible {
                reason: ActiveWorkIneligibleReason::NotAdmitted,
            },
            Self::Blocked => ActiveWorkEligibility::Ineligible {
                reason: ActiveWorkIneligibleReason::Blocked,
            },
            Self::Done | Self::Cancelled => ActiveWorkEligibility::Ineligible {
                reason: ActiveWorkIneligibleReason::Terminal,
            },
        }
    }

    /// Decide whether one explicit Checkpoint recommendation may be accepted.
    #[must_use]
    pub fn can_accept_recommendation(self, recommended: Self) -> bool {
        if self == recommended {
            return true;
        }
        matches!(
            self,
            Self::Todo | Self::InProgress | Self::InReview | Self::Blocked
        ) && recommended.is_reconciliation_target()
    }

    /// Identify state recommendations handled by Checkpoint reconciliation.
    #[must_use]
    pub const fn is_reconciliation_target(self) -> bool {
        matches!(
            self,
            Self::InProgress | Self::InReview | Self::Blocked | Self::Done
        )
    }
}

impl TryFrom<&str> for WorkItemState {
    type Error = UnknownWorkItemState;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "backlog" => Ok(Self::Backlog),
            "todo" => Ok(Self::Todo),
            "in_progress" => Ok(Self::InProgress),
            "in_review" => Ok(Self::InReview),
            "blocked" => Ok(Self::Blocked),
            "done" => Ok(Self::Done),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(UnknownWorkItemState),
        }
    }
}

impl fmt::Display for WorkItemState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Human-facing project health computed from Board-owned coordination state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectHealth {
    Healthy,
    Blocked,
    Idle,
}

impl ProjectHealth {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Blocked => "blocked",
            Self::Idle => "idle",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Healthy => "Healthy",
            Self::Blocked => "Needs attention",
            Self::Idle => "Idle",
        }
    }
}

impl TryFrom<&str> for ProjectHealth {
    type Error = UnknownProjectHealth;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "healthy" => Ok(Self::Healthy),
            "blocked" => Ok(Self::Blocked),
            "idle" => Ok(Self::Idle),
            _ => Err(UnknownProjectHealth),
        }
    }
}

/// Stored health value violated the domain vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownProjectHealth;

impl fmt::Display for UnknownProjectHealth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unknown project health")
    }
}

impl std::error::Error for UnknownProjectHealth {}

/// Stored Work item state violated the domain vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownWorkItemState;

impl fmt::Display for UnknownWorkItemState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unknown Work item state")
    }
}

impl std::error::Error for UnknownWorkItemState {}

/// Board-owned result of deciding whether work may reference one Work item.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ActiveWorkEligibility {
    Eligible,
    Ineligible { reason: ActiveWorkIneligibleReason },
}

/// Stable reasons that a Work item cannot be selected as active work.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveWorkIneligibleReason {
    NotAdmitted,
    Blocked,
    Terminal,
}

/// Attributed active-work assessment returned by Board.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActiveWorkAssessment {
    pub project_id: String,
    pub work_item_id: String,
    pub state: WorkItemState,
    pub eligibility: ActiveWorkEligibility,
}

/// Capture origin presented in the Activity timeline.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointSource {
    Runner,
    Mcp,
    ManualCli,
    CodexStopHook,
    GitPostCommit,
}

impl CheckpointSource {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Runner => "Runner",
            Self::Mcp => "MCP",
            Self::ManualCli => "Manual",
            Self::CodexStopHook => "Codex",
            Self::GitPostCommit => "Git",
        }
    }
}

/// Progress outcome presented independently from Work item state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointOutcome {
    Progress,
    Completed,
    NeedsReview,
    Blocked,
    Failed,
    NoAction,
}

impl CheckpointOutcome {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Progress => "Progress",
            Self::Completed => "Completed",
            Self::NeedsReview => "Needs review",
            Self::Blocked => "Blocked",
            Self::Failed => "Failed",
            Self::NoAction => "No action",
        }
    }
}

/// Current projection result for one Progress Checkpoint destination.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointDeliveryStatus {
    Pending,
    Synced,
    Conflict,
    Failed,
}

impl CheckpointDeliveryStatus {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Synced => "Synced",
            Self::Conflict => "Conflict",
            Self::Failed => "Failed",
        }
    }

    #[must_use]
    pub const fn needs_attention(self) -> bool {
        matches!(self, Self::Conflict | Self::Failed)
    }
}

/// One destination result shown with a Progress Checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointDelivery {
    pub destination_id: String,
    pub status: CheckpointDeliveryStatus,
    pub attempts: u32,
    pub last_error: Option<String>,
}

/// Final Board-owned judgment over one Checkpoint recommendation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationDecision {
    Accepted,
    Dismissed,
}

impl ReconciliationDecision {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Dismissed => "dismissed",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Accepted => "Accepted",
            Self::Dismissed => "Dismissed",
        }
    }
}

impl TryFrom<&str> for ReconciliationDecision {
    type Error = UnknownReconciliationDecision;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "accepted" => Ok(Self::Accepted),
            "dismissed" => Ok(Self::Dismissed),
            _ => Err(UnknownReconciliationDecision),
        }
    }
}

/// Stored reconciliation result displayed with one Activity timeline entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointReconciliation {
    pub decision: ReconciliationDecision,
    pub recommended_state: WorkItemState,
    pub previous_state: WorkItemState,
    pub resulting_state: WorkItemState,
}

/// Explicit user intent passed to Board's reconciliation Module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationRequest {
    pub checkpoint_id: String,
    pub project_id: String,
    pub work_item_id: String,
    pub recommended_state: WorkItemState,
    pub decision: ReconciliationDecision,
}

/// Durable result from one reconciliation attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationReceipt {
    pub checkpoint_id: String,
    pub duplicate: bool,
    pub reconciliation: CheckpointReconciliation,
}

/// Final Board-owned Work item association for a project-only Checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointAttachment {
    pub work_item_id: String,
}

/// Explicit user intent passed to Board's Checkpoint attachment Module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachmentRequest {
    pub checkpoint_id: String,
    pub project_id: String,
    pub checkpoint_work_item_id: Option<String>,
    pub target: AttachmentTarget,
}

/// Human-selected destination for one Activity Inbox Checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttachmentTarget {
    Existing { work_item_id: String },
    New { work_item_id: String, title: String },
}

impl AttachmentTarget {
    /// Return the stable Work item identity used by the final attachment.
    #[must_use]
    pub fn work_item_id(&self) -> &str {
        match self {
            Self::Existing { work_item_id } | Self::New { work_item_id, .. } => work_item_id,
        }
    }
}

/// Durable result from one Checkpoint attachment attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachmentReceipt {
    pub checkpoint_id: String,
    pub duplicate: bool,
    pub created_work_item: bool,
    pub attachment: CheckpointAttachment,
}

/// Stored reconciliation decision violated the domain vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownReconciliationDecision;

impl fmt::Display for UnknownReconciliationDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unknown reconciliation decision")
    }
}

impl std::error::Error for UnknownReconciliationDecision {}

/// One accepted Core checkpoint adapted for Board presentation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgressActivity {
    pub checkpoint_id: String,
    pub recorded_at: String,
    pub project_id: String,
    pub work_item_id: Option<String>,
    pub source: CheckpointSource,
    pub outcome: CheckpointOutcome,
    pub summary: String,
    pub recommended_state: Option<WorkItemState>,
    pub deliveries: Vec<CheckpointDelivery>,
    pub attachment: Option<CheckpointAttachment>,
    pub reconciliation: Option<CheckpointReconciliation>,
}

impl ProgressActivity {
    /// Resolve the original Core link or a later Board-owned attachment.
    #[must_use]
    pub fn effective_work_item_id(&self) -> Option<&str> {
        self.work_item_id.as_deref().or_else(|| {
            self.attachment
                .as_ref()
                .map(|attachment| attachment.work_item_id.as_str())
        })
    }

    /// Identify project-only activity still waiting for a human attachment.
    #[must_use]
    pub fn is_inbox(&self) -> bool {
        self.effective_work_item_id().is_none()
    }
}

/// Bounded Board read model derived from Core-owned Progress Checkpoints.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActivityTimeline {
    pub activities: Vec<ProgressActivity>,
    pub has_older: bool,
}

impl ActivityTimeline {
    #[must_use]
    pub fn delivery_issues(&self) -> u32 {
        self.activities
            .iter()
            .flat_map(|activity| &activity.deliveries)
            .filter(|delivery| delivery.status.needs_attention())
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    /// Count project-only Checkpoints awaiting a Work item attachment.
    #[must_use]
    pub fn inbox_count(&self) -> u32 {
        self.activities
            .iter()
            .filter(|activity| activity.is_inbox())
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }
}

/// Existing Board Work item available as an Activity Inbox target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkItemSummary {
    pub id: String,
    pub project_id: String,
    pub title: String,
    /// Lower values are considered first by deterministic selection policies.
    pub priority: u32,
    pub state: WorkItemState,
}

/// Explicit human intent to change one Board-owned Work item state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkItemTransitionRequest {
    pub project_id: String,
    pub work_item_id: String,
    pub expected_state: WorkItemState,
    pub target_state: WorkItemState,
}

/// Durable result of one explicit Work item transition attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkItemTransitionReceipt {
    pub work_item_id: String,
    pub previous_state: WorkItemState,
    pub resulting_state: WorkItemState,
    pub changed: bool,
}

/// Counts needed by the portfolio screen without exposing storage rows.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkItemCounts {
    pub total: u32,
    pub todo: u32,
    pub in_progress: u32,
    pub in_review: u32,
    pub blocked: u32,
    pub done: u32,
}

/// One project card on the portfolio screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub health: ProjectHealth,
    pub execution_cap: u32,
    pub work_items: WorkItemCounts,
}

/// Complete bounded read model consumed by the initial Board screen.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PortfolioSnapshot {
    pub projects: Vec<ProjectSummary>,
}

impl PortfolioSnapshot {
    #[must_use]
    pub fn active_runs(&self) -> u32 {
        self.projects
            .iter()
            .map(|project| project.work_items.in_progress)
            .sum()
    }

    #[must_use]
    pub fn blocked_items(&self) -> u32 {
        self.projects
            .iter()
            .map(|project| project.work_items.blocked)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portfolio_totals_are_derived_from_project_summaries() {
        let snapshot = PortfolioSnapshot {
            projects: vec![ProjectSummary {
                id: "core".to_owned(),
                name: "Gareji Core".to_owned(),
                health: ProjectHealth::Healthy,
                execution_cap: 1,
                work_items: WorkItemCounts {
                    in_progress: 2,
                    blocked: 1,
                    ..WorkItemCounts::default()
                },
            }],
        };

        assert_eq!(snapshot.active_runs(), 2);
        assert_eq!(snapshot.blocked_items(), 1);
    }

    #[test]
    fn active_work_eligibility_is_owned_by_work_item_state() {
        for eligible in [
            WorkItemState::Todo,
            WorkItemState::InProgress,
            WorkItemState::InReview,
        ] {
            assert_eq!(
                eligible.active_work_eligibility(),
                ActiveWorkEligibility::Eligible
            );
        }
        assert_eq!(
            WorkItemState::Backlog.active_work_eligibility(),
            ActiveWorkEligibility::Ineligible {
                reason: ActiveWorkIneligibleReason::NotAdmitted
            }
        );
        assert_eq!(
            WorkItemState::Blocked.active_work_eligibility(),
            ActiveWorkEligibility::Ineligible {
                reason: ActiveWorkIneligibleReason::Blocked
            }
        );
        for terminal in [WorkItemState::Done, WorkItemState::Cancelled] {
            assert_eq!(
                terminal.active_work_eligibility(),
                ActiveWorkEligibility::Ineligible {
                    reason: ActiveWorkIneligibleReason::Terminal
                }
            );
        }
    }

    #[test]
    fn activity_timeline_counts_delivery_issues_only() {
        let timeline = ActivityTimeline {
            activities: vec![ProgressActivity {
                checkpoint_id: "cp-1".to_owned(),
                recorded_at: "2026-07-17T12:00:00+09:00".to_owned(),
                project_id: "core".to_owned(),
                work_item_id: Some("CORE-1".to_owned()),
                source: CheckpointSource::Runner,
                outcome: CheckpointOutcome::Progress,
                summary: "Made progress".to_owned(),
                recommended_state: None,
                deliveries: vec![
                    CheckpointDelivery {
                        destination_id: "notes".to_owned(),
                        status: CheckpointDeliveryStatus::Synced,
                        attempts: 1,
                        last_error: None,
                    },
                    CheckpointDelivery {
                        destination_id: "wiki".to_owned(),
                        status: CheckpointDeliveryStatus::Failed,
                        attempts: 1,
                        last_error: Some("unavailable".to_owned()),
                    },
                ],
                attachment: None,
                reconciliation: None,
            }],
            has_older: false,
        };

        assert_eq!(timeline.delivery_issues(), 1);
        assert_eq!(timeline.inbox_count(), 0);
    }

    #[test]
    fn attachment_resolves_an_inbox_activity_without_rewriting_its_core_link() {
        let mut activity = ProgressActivity {
            checkpoint_id: "cp-inbox".to_owned(),
            recorded_at: "2026-07-17T12:00:00+09:00".to_owned(),
            project_id: "core".to_owned(),
            work_item_id: None,
            source: CheckpointSource::Mcp,
            outcome: CheckpointOutcome::Progress,
            summary: "Made progress".to_owned(),
            recommended_state: None,
            deliveries: Vec::new(),
            attachment: None,
            reconciliation: None,
        };

        assert!(activity.is_inbox());
        activity.attachment = Some(CheckpointAttachment {
            work_item_id: "CORE-1".to_owned(),
        });
        assert_eq!(activity.work_item_id, None);
        assert_eq!(activity.effective_work_item_id(), Some("CORE-1"));
        assert!(!activity.is_inbox());
    }

    #[test]
    fn checkpoint_recommendations_use_the_safe_reconciliation_subset() {
        assert!(WorkItemState::Todo.can_accept_recommendation(WorkItemState::InProgress));
        assert!(WorkItemState::InProgress.can_accept_recommendation(WorkItemState::Done));
        assert!(WorkItemState::Blocked.can_accept_recommendation(WorkItemState::InReview));
        assert!(WorkItemState::Done.can_accept_recommendation(WorkItemState::Done));

        assert!(!WorkItemState::Backlog.can_accept_recommendation(WorkItemState::InProgress));
        assert!(!WorkItemState::InReview.can_accept_recommendation(WorkItemState::Todo));
        assert!(!WorkItemState::Done.can_accept_recommendation(WorkItemState::InReview));
        assert!(!WorkItemState::Cancelled.can_accept_recommendation(WorkItemState::Done));
    }
}
