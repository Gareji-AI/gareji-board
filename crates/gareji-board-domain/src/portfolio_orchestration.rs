use std::collections::{HashSet, VecDeque};
use std::fmt;

use serde::{Deserialize, Serialize};

/// Immutable Board-owned routing across managed projects.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PortfolioOrchestrationRevision {
    pub orchestration_id: String,
    pub revision_id: String,
    pub name: String,
    pub schedule: PortfolioSchedule,
    pub entry_node_id: String,
    pub nodes: Vec<PortfolioNode>,
    pub routes: Vec<PortfolioRoute>,
}

impl PortfolioOrchestrationRevision {
    /// Validate identity, reachability, deterministic routing, and authority.
    ///
    /// Cycles are permitted because a cadence may revisit projects on later ticks.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation error for malformed or ambiguous orchestration.
    pub fn validate(&self) -> Result<(), PortfolioOrchestrationValidationError> {
        self.validate_editable_structure()?;
        let has_terminal = self
            .nodes
            .iter()
            .any(|node| matches!(node.kind, PortfolioNodeKind::Terminal));
        if !has_terminal {
            return Err(PortfolioOrchestrationValidationError::MissingTerminal);
        }
        self.validate_reachability()
    }

    /// Validate identities, node contracts, schedule, routes, and authority while editing.
    ///
    /// Unlike [`Self::validate`], this permits temporarily unreachable nodes and a
    /// temporarily missing Terminal. Stored revisions must still pass complete validation.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when one draft edit violates the local contract.
    pub fn validate_editable_structure(&self) -> Result<(), PortfolioOrchestrationValidationError> {
        validate_identifier(&self.orchestration_id, "orchestration_id")?;
        validate_identifier(&self.revision_id, "revision_id")?;
        validate_identifier(&self.entry_node_id, "entry_node_id")?;
        if self.name.is_empty() || self.name.trim() != self.name || self.name.chars().count() > 128
        {
            return Err(PortfolioOrchestrationValidationError::InvalidName);
        }
        self.schedule.validate()?;
        if self.nodes.is_empty() {
            return Err(PortfolioOrchestrationValidationError::MissingNode);
        }

        let mut node_ids = HashSet::with_capacity(self.nodes.len());
        for node in &self.nodes {
            validate_identifier(&node.id, "node_id")?;
            if !node_ids.insert(node.id.as_str()) {
                return Err(PortfolioOrchestrationValidationError::DuplicateNode {
                    node_id: node.id.clone(),
                });
            }
            match &node.kind {
                PortfolioNodeKind::ProjectSelector { selector } => selector.validate()?,
                PortfolioNodeKind::ProjectInvocation { project_id } => {
                    validate_identifier(project_id, "project_id")?;
                }
                PortfolioNodeKind::PostAction { .. } | PortfolioNodeKind::Terminal => {}
            }
        }
        if !node_ids.contains(self.entry_node_id.as_str()) {
            return Err(PortfolioOrchestrationValidationError::NodeNotFound {
                node_id: self.entry_node_id.clone(),
            });
        }
        self.validate_routes(&node_ids)?;
        Ok(())
    }

    /// Return the scheduled entry node after validating the revision.
    ///
    /// # Errors
    ///
    /// Returns the same bounded validation errors as [`Self::validate`].
    pub fn entry_node(&self) -> Result<&PortfolioNode, PortfolioOrchestrationValidationError> {
        self.validate()?;
        self.nodes
            .iter()
            .find(|node| node.id == self.entry_node_id)
            .ok_or_else(|| PortfolioOrchestrationValidationError::NodeNotFound {
                node_id: self.entry_node_id.clone(),
            })
    }

    /// Resolve one node in this immutable revision.
    #[must_use]
    pub fn node(&self, node_id: &str) -> Option<&PortfolioNode> {
        self.nodes.iter().find(|node| node.id == node_id)
    }

    /// Resolve the deterministic route emitted by one node and signal.
    #[must_use]
    pub fn route(&self, source_node_id: &str, signal: PortfolioSignal) -> Option<&PortfolioRoute> {
        self.routes
            .iter()
            .find(|route| route.source_node_id == source_node_id && route.signal == signal)
    }

    fn validate_routes(
        &self,
        node_ids: &HashSet<&str>,
    ) -> Result<(), PortfolioOrchestrationValidationError> {
        let mut route_ids = HashSet::with_capacity(self.routes.len());
        let mut route_keys = HashSet::with_capacity(self.routes.len());
        for route in &self.routes {
            validate_identifier(&route.id, "route_id")?;
            if !route_ids.insert(route.id.as_str()) {
                return Err(PortfolioOrchestrationValidationError::DuplicateRoute {
                    route_id: route.id.clone(),
                });
            }
            for node_id in [&route.source_node_id, &route.destination_node_id] {
                if !node_ids.contains(node_id.as_str()) {
                    return Err(PortfolioOrchestrationValidationError::NodeNotFound {
                        node_id: node_id.clone(),
                    });
                }
            }
            if !route_keys.insert((route.source_node_id.as_str(), route.signal)) {
                return Err(PortfolioOrchestrationValidationError::AmbiguousRoute {
                    source_node_id: route.source_node_id.clone(),
                    signal: route.signal,
                });
            }
            let Some(source) = self
                .nodes
                .iter()
                .find(|node| node.id == route.source_node_id)
            else {
                return Err(PortfolioOrchestrationValidationError::NodeNotFound {
                    node_id: route.source_node_id.clone(),
                });
            };
            validate_route_authority(source, route)?;
        }
        Ok(())
    }

    fn validate_reachability(&self) -> Result<(), PortfolioOrchestrationValidationError> {
        let terminal_ids = self
            .nodes
            .iter()
            .filter_map(|node| {
                matches!(node.kind, PortfolioNodeKind::Terminal).then_some(node.id.as_str())
            })
            .collect::<HashSet<_>>();
        if let Some(route) = self
            .routes
            .iter()
            .find(|route| terminal_ids.contains(route.source_node_id.as_str()))
        {
            return Err(PortfolioOrchestrationValidationError::TerminalHasRoute {
                node_id: route.source_node_id.clone(),
            });
        }

        let mut reachable = HashSet::from([self.entry_node_id.as_str()]);
        let mut queue = VecDeque::from([self.entry_node_id.as_str()]);
        while let Some(source_node_id) = queue.pop_front() {
            for destination_node_id in self
                .routes
                .iter()
                .filter(|route| route.source_node_id == source_node_id)
                .map(|route| route.destination_node_id.as_str())
            {
                if reachable.insert(destination_node_id) {
                    queue.push_back(destination_node_id);
                }
            }
        }
        if let Some(node) = self
            .nodes
            .iter()
            .find(|node| !reachable.contains(node.id.as_str()))
        {
            return Err(PortfolioOrchestrationValidationError::UnreachableNode {
                node_id: node.id.clone(),
            });
        }
        Ok(())
    }
}

/// Cadence configuration. An enabled interval is configuration only until a
/// local scheduler Adapter creates a Portfolio Run.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PortfolioSchedule {
    Manual,
    Interval { every_minutes: u32, enabled: bool },
}

impl PortfolioSchedule {
    fn validate(self) -> Result<(), PortfolioOrchestrationValidationError> {
        if let Self::Interval { every_minutes, .. } = self
            && !(5..=10_080).contains(&every_minutes)
        {
            return Err(PortfolioOrchestrationValidationError::InvalidInterval);
        }
        Ok(())
    }

    /// Return the enabled cadence in seconds. Manual and disabled schedules
    /// deliberately have no automatic wake-up.
    #[must_use]
    pub const fn interval_seconds(self) -> Option<i64> {
        match self {
            Self::Interval {
                every_minutes,
                enabled: true,
            } => Some(every_minutes as i64 * 60),
            Self::Manual | Self::Interval { enabled: false, .. } => None,
        }
    }
}

/// Runtime control over automatic ticks for one immutable Portfolio revision.
/// It does not modify the revision or the current Run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioScheduleControl {
    pub orchestration_id: String,
    pub revision_id: String,
    pub automatic_ticks_enabled: bool,
}

impl PortfolioScheduleControl {
    /// Derive the initial runtime control from immutable schedule configuration.
    #[must_use]
    pub fn from_revision(revision: &PortfolioOrchestrationRevision) -> Self {
        Self {
            orchestration_id: revision.orchestration_id.clone(),
            revision_id: revision.revision_id.clone(),
            automatic_ticks_enabled: revision.schedule.interval_seconds().is_some(),
        }
    }
}

/// Optimistic intent to pause or resume automatic Portfolio ticks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioScheduleControlSaveRequest {
    pub expected: PortfolioScheduleControl,
    pub target: PortfolioScheduleControl,
}

/// Durable result of one schedule-control change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioScheduleControlSaveReceipt {
    pub previous: PortfolioScheduleControl,
    pub resulting: PortfolioScheduleControl,
    pub changed: bool,
}

/// One stage in portfolio-wide coordination.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PortfolioNode {
    pub id: String,
    pub kind: PortfolioNodeKind,
}

/// Bounded behavior of one portfolio orchestration node.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PortfolioNodeKind {
    /// Select one eligible Work item at tick time without pinning the graph to
    /// a particular Board project.
    ProjectSelector {
        selector: PortfolioProjectSelector,
    },
    /// Compatibility-only representation for already persisted v1 graphs.
    /// New revisions use [`Self::ProjectSelector`].
    ProjectInvocation {
        project_id: String,
    },
    PostAction {
        action: PortfolioPostAction,
    },
    Terminal,
}

/// Project set considered by one Project Selector node.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PortfolioProjectSelector {
    AllManaged,
    Include { project_ids: Vec<String> },
}

impl PortfolioProjectSelector {
    fn validate(&self) -> Result<(), PortfolioOrchestrationValidationError> {
        let Self::Include { project_ids } = self else {
            return Ok(());
        };
        if project_ids.is_empty() {
            return Err(PortfolioOrchestrationValidationError::EmptyProjectSelection);
        }
        let mut unique = HashSet::with_capacity(project_ids.len());
        for project_id in project_ids {
            validate_identifier(project_id, "project_id")?;
            if !unique.insert(project_id.as_str()) {
                return Err(
                    PortfolioOrchestrationValidationError::DuplicateSelectedProject {
                        project_id: project_id.clone(),
                    },
                );
            }
        }
        Ok(())
    }

    /// Decide whether a managed project belongs to this selector.
    #[must_use]
    pub fn includes(&self, project_id: &str) -> bool {
        match self {
            Self::AllManaged => true,
            Self::Include { project_ids } => project_ids.iter().any(|id| id == project_id),
        }
    }
}

/// Explicit non-project action available after a project step.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortfolioPostAction {
    RecordSummary,
    RequestApproval,
}

/// Explicit human outcome for a waiting Portfolio approval node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortfolioApprovalDecision {
    Approved,
    Rejected,
}

impl PortfolioApprovalDecision {
    #[must_use]
    pub const fn signal(self) -> PortfolioSignal {
        match self {
            Self::Approved => PortfolioSignal::Approved,
            Self::Rejected => PortfolioSignal::Rejected,
        }
    }
}

/// Deterministic edge across portfolio orchestration nodes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PortfolioRoute {
    pub id: String,
    pub source_node_id: String,
    pub destination_node_id: String,
    pub signal: PortfolioSignal,
}

/// Bounded result used to select one portfolio route.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortfolioSignal {
    Completed,
    NoCandidate,
    NeedsAttention,
    Failed,
    Approved,
    Rejected,
    Manual,
}

impl PortfolioSignal {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::NoCandidate => "no_candidate",
            Self::NeedsAttention => "needs_attention",
            Self::Failed => "failed",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Manual => "manual",
        }
    }
}

impl TryFrom<&str> for PortfolioSignal {
    type Error = PortfolioSignalParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "completed" => Ok(Self::Completed),
            "no_candidate" => Ok(Self::NoCandidate),
            "needs_attention" => Ok(Self::NeedsAttention),
            "failed" => Ok(Self::Failed),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "manual" => Ok(Self::Manual),
            _ => Err(PortfolioSignalParseError),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortfolioSignalParseError;

impl fmt::Display for PortfolioSignalParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unknown Portfolio signal")
    }
}

impl std::error::Error for PortfolioSignalParseError {}

fn validate_route_authority(
    source: &PortfolioNode,
    route: &PortfolioRoute,
) -> Result<(), PortfolioOrchestrationValidationError> {
    let valid = match source.kind {
        PortfolioNodeKind::ProjectSelector { .. } | PortfolioNodeKind::ProjectInvocation { .. } => {
            matches!(
                route.signal,
                PortfolioSignal::Completed
                    | PortfolioSignal::NoCandidate
                    | PortfolioSignal::NeedsAttention
                    | PortfolioSignal::Failed
                    | PortfolioSignal::Manual
            )
        }
        PortfolioNodeKind::PostAction {
            action: PortfolioPostAction::RequestApproval,
        } => matches!(
            route.signal,
            PortfolioSignal::Approved | PortfolioSignal::Rejected
        ),
        PortfolioNodeKind::PostAction {
            action: PortfolioPostAction::RecordSummary,
        } => matches!(
            route.signal,
            PortfolioSignal::Completed | PortfolioSignal::Failed | PortfolioSignal::Manual
        ),
        PortfolioNodeKind::Terminal => false,
    };
    if valid {
        Ok(())
    } else {
        Err(PortfolioOrchestrationValidationError::InvalidRouteSignal {
            node_id: source.id.clone(),
            signal: route.signal,
        })
    }
}

fn validate_identifier(
    value: &str,
    field: &'static str,
) -> Result<(), PortfolioOrchestrationValidationError> {
    let length = value.chars().count();
    let valid_first = value
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_lowercase());
    let valid_last = value
        .chars()
        .next_back()
        .is_some_and(|character| character.is_ascii_lowercase() || character.is_ascii_digit());
    let valid_characters = value.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '-' | '_')
    });
    if length == 0 || length > 64 || !valid_first || !valid_last || !valid_characters {
        return Err(PortfolioOrchestrationValidationError::InvalidIdentifier { field });
    }
    Ok(())
}

/// Bounded reason why a portfolio orchestration revision is invalid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortfolioOrchestrationValidationError {
    InvalidIdentifier {
        field: &'static str,
    },
    InvalidName,
    InvalidInterval,
    MissingNode,
    MissingTerminal,
    EmptyProjectSelection,
    DuplicateSelectedProject {
        project_id: String,
    },
    DuplicateNode {
        node_id: String,
    },
    DuplicateRoute {
        route_id: String,
    },
    NodeNotFound {
        node_id: String,
    },
    AmbiguousRoute {
        source_node_id: String,
        signal: PortfolioSignal,
    },
    InvalidRouteSignal {
        node_id: String,
        signal: PortfolioSignal,
    },
    TerminalHasRoute {
        node_id: String,
    },
    UnreachableNode {
        node_id: String,
    },
}

/// Durable lifecycle of one Portfolio Run pinned to an immutable revision.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortfolioRunStatus {
    Active,
    WaitingApproval,
    Paused,
    Completed,
}

impl PortfolioRunStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::WaitingApproval => "waiting_approval",
            Self::Paused => "paused",
            Self::Completed => "completed",
        }
    }
}

impl TryFrom<&str> for PortfolioRunStatus {
    type Error = PortfolioRunStatusParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "active" => Ok(Self::Active),
            "waiting_approval" => Ok(Self::WaitingApproval),
            "paused" => Ok(Self::Paused),
            "completed" => Ok(Self::Completed),
            _ => Err(PortfolioRunStatusParseError),
        }
    }
}

/// Stored Portfolio Run pinned to one graph revision and current node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioRun {
    pub run_id: String,
    pub orchestration_id: String,
    pub revision_id: String,
    pub current_node_id: String,
    pub status: PortfolioRunStatus,
    pub completed_steps: u32,
    pub next_tick_at_epoch_seconds: Option<i64>,
}

impl PortfolioRun {
    /// An automatic scheduler may only advance an active run once it is due.
    #[must_use]
    pub fn is_due(&self, now_epoch_seconds: i64) -> bool {
        self.status == PortfolioRunStatus::Active
            && self
                .next_tick_at_epoch_seconds
                .is_some_and(|due| due <= now_epoch_seconds)
    }
}

/// Append-only observation produced by exactly one Portfolio tick.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioRunStep {
    pub run_id: String,
    pub sequence: u32,
    pub node_id: String,
    pub signal: Option<PortfolioSignal>,
    pub destination_node_id: Option<String>,
    pub selected_project_id: Option<String>,
    pub recorded_at_epoch_seconds: i64,
}

/// Stored Portfolio Run status used an unknown value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortfolioRunStatusParseError;

impl fmt::Display for PortfolioRunStatusParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unknown Portfolio Run status")
    }
}

impl std::error::Error for PortfolioRunStatusParseError {}

impl fmt::Display for PortfolioOrchestrationValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid Portfolio orchestration revision: {self:?}"
        )
    }
}

impl std::error::Error for PortfolioOrchestrationValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_reachable_cross_project_revision() {
        let revision = sample_revision();

        assert_eq!(revision.entry_node().unwrap().id, "board");
        assert!(revision.validate().is_ok());
    }

    #[test]
    fn rejects_approval_authority_and_unreachable_projects() {
        let mut invalid_authority = sample_revision();
        invalid_authority.routes[0].signal = PortfolioSignal::Approved;
        assert!(matches!(
            invalid_authority.validate(),
            Err(PortfolioOrchestrationValidationError::InvalidRouteSignal { .. })
        ));

        let mut unreachable = sample_revision();
        unreachable.routes.remove(0);
        assert!(matches!(
            unreachable.validate(),
            Err(PortfolioOrchestrationValidationError::UnreachableNode { .. })
        ));
    }

    fn sample_revision() -> PortfolioOrchestrationRevision {
        PortfolioOrchestrationRevision {
            orchestration_id: "daily-portfolio".to_owned(),
            revision_id: "v1".to_owned(),
            name: "Daily portfolio".to_owned(),
            schedule: PortfolioSchedule::Interval {
                every_minutes: 60,
                enabled: false,
            },
            entry_node_id: "board".to_owned(),
            nodes: vec![
                PortfolioNode {
                    id: "board".to_owned(),
                    kind: PortfolioNodeKind::ProjectInvocation {
                        project_id: "gareji-board".to_owned(),
                    },
                },
                PortfolioNode {
                    id: "summary".to_owned(),
                    kind: PortfolioNodeKind::PostAction {
                        action: PortfolioPostAction::RecordSummary,
                    },
                },
                PortfolioNode {
                    id: "finish".to_owned(),
                    kind: PortfolioNodeKind::Terminal,
                },
            ],
            routes: vec![
                PortfolioRoute {
                    id: "board-completed".to_owned(),
                    source_node_id: "board".to_owned(),
                    destination_node_id: "summary".to_owned(),
                    signal: PortfolioSignal::Completed,
                },
                PortfolioRoute {
                    id: "summary-recorded".to_owned(),
                    source_node_id: "summary".to_owned(),
                    destination_node_id: "finish".to_owned(),
                    signal: PortfolioSignal::Completed,
                },
            ],
        }
    }
}
