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
}
