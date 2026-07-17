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
}
