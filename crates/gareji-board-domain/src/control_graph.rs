use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Immutable Board-owned organization and routing definition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ControlGraphRevision {
    pub graph_id: String,
    pub revision_id: String,
    pub entries: Vec<GraphEntry>,
    pub nodes: Vec<ControlNode>,
    pub routes: Vec<ControlRoute>,
    pub anchors: Vec<GraphAnchor>,
}

/// Board-local presentation state for one reusable Control graph.
///
/// This intentionally stays separate from immutable execution revisions: moving
/// a node in the editor must never change routing or approval semantics.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphCanvasLayout {
    pub graph_id: String,
    pub positions: Vec<GraphCanvasNodePosition>,
}

impl GraphCanvasLayout {
    /// Validate stable identities and bounded finite canvas coordinates.
    ///
    /// # Errors
    ///
    /// Returns a graph validation error for malformed or duplicate positions.
    pub fn validate(&self) -> Result<(), ControlGraphValidationError> {
        validate_identifier(&self.graph_id, "graph_id")?;
        let mut node_ids = HashSet::with_capacity(self.positions.len());
        for position in &self.positions {
            validate_identifier(&position.node_id, "node_id")?;
            if !node_ids.insert(position.node_id.as_str()) {
                return Err(ControlGraphValidationError::DuplicateNode {
                    node_id: position.node_id.clone(),
                });
            }
            if !position.x.is_finite()
                || !position.y.is_finite()
                || !(0.0..=100_000.0).contains(&position.x)
                || !(0.0..=100_000.0).contains(&position.y)
            {
                return Err(ControlGraphValidationError::InvalidCanvasPosition {
                    node_id: position.node_id.clone(),
                });
            }
        }
        Ok(())
    }
}

/// One freeform node position in a Graph canvas layout.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphCanvasNodePosition {
    pub node_id: String,
    pub x: f64,
    pub y: f64,
}

impl ControlGraphRevision {
    /// Validate graph identity, grounding, and deterministic references.
    ///
    /// Cycles are intentionally permitted because focused Agent loops may revisit
    /// review or correction stages.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation error when an identity, reference, anchor,
    /// terminal rule, or deterministic route constraint is invalid.
    pub fn validate(&self) -> Result<(), ControlGraphValidationError> {
        validate_identifier(&self.graph_id, "graph_id")?;
        validate_identifier(&self.revision_id, "revision_id")?;
        if self.entries.is_empty() {
            return Err(ControlGraphValidationError::MissingEntry);
        }
        if self.nodes.is_empty() {
            return Err(ControlGraphValidationError::MissingNode);
        }
        if self.anchors.is_empty() {
            return Err(ControlGraphValidationError::MissingAnchor);
        }

        let mut node_ids = HashSet::with_capacity(self.nodes.len());
        for node in &self.nodes {
            validate_identifier(&node.id, "node_id")?;
            if !node_ids.insert(node.id.as_str()) {
                return Err(ControlGraphValidationError::DuplicateNode {
                    node_id: node.id.clone(),
                });
            }
            if let ControlNodeKind::AgentLoop { agent_profile_id } = &node.kind {
                validate_identifier(agent_profile_id, "agent_profile_id")?;
            }
        }

        let mut entry_ids = HashSet::with_capacity(self.entries.len());
        for entry in &self.entries {
            validate_identifier(&entry.id, "entry_id")?;
            if !entry_ids.insert(entry.id.as_str()) {
                return Err(ControlGraphValidationError::DuplicateEntry {
                    entry_id: entry.id.clone(),
                });
            }
            if !node_ids.contains(entry.node_id.as_str()) {
                return Err(ControlGraphValidationError::NodeNotFound {
                    node_id: entry.node_id.clone(),
                });
            }
        }

        let mut route_ids = HashSet::with_capacity(self.routes.len());
        let mut route_keys = HashSet::with_capacity(self.routes.len());
        for route in &self.routes {
            validate_identifier(&route.id, "route_id")?;
            if !route_ids.insert(route.id.as_str()) {
                return Err(ControlGraphValidationError::DuplicateRoute {
                    route_id: route.id.clone(),
                });
            }
            for node_id in [&route.source_node_id, &route.destination_node_id] {
                if !node_ids.contains(node_id.as_str()) {
                    return Err(ControlGraphValidationError::NodeNotFound {
                        node_id: node_id.clone(),
                    });
                }
            }
            if !route_keys.insert((route.source_node_id.as_str(), route.signal)) {
                return Err(ControlGraphValidationError::AmbiguousRoute {
                    source_node_id: route.source_node_id.clone(),
                    signal: route.signal,
                });
            }
            validate_route_authority(&self.nodes, route)?;
        }

        let terminal_nodes: HashSet<&str> = self
            .nodes
            .iter()
            .filter_map(|node| {
                matches!(node.kind, ControlNodeKind::Terminal).then_some(node.id.as_str())
            })
            .collect();
        if let Some(route) = self
            .routes
            .iter()
            .find(|route| terminal_nodes.contains(route.source_node_id.as_str()))
        {
            return Err(ControlGraphValidationError::TerminalHasRoute {
                node_id: route.source_node_id.clone(),
            });
        }

        let mut anchor_ids = HashSet::with_capacity(self.anchors.len());
        for anchor in &self.anchors {
            validate_identifier(&anchor.id, "anchor_id")?;
            let length = anchor.description.chars().count();
            if length == 0 || length > 512 || anchor.description.trim() != anchor.description {
                return Err(ControlGraphValidationError::InvalidAnchorDescription);
            }
            if !anchor_ids.insert(anchor.id.as_str()) {
                return Err(ControlGraphValidationError::DuplicateAnchor {
                    anchor_id: anchor.id.clone(),
                });
            }
        }
        Ok(())
    }

    /// Select the one declared route for a current node and observed signal.
    ///
    /// An optional proposal can name that route, but cannot introduce a different
    /// edge or destination.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the graph is invalid, the current node or
    /// route does not exist, or the proposal differs from deterministic policy.
    pub fn select_route(
        &self,
        current_node_id: &str,
        signal: ControlSignal,
        proposed_route_id: Option<&str>,
    ) -> Result<SelectedControlRoute, ControlRouteSelectionError> {
        self.validate()
            .map_err(ControlRouteSelectionError::InvalidGraph)?;
        if !self.nodes.iter().any(|node| node.id == current_node_id) {
            return Err(ControlRouteSelectionError::NodeNotFound {
                node_id: current_node_id.to_owned(),
            });
        }
        let route = self
            .routes
            .iter()
            .find(|route| route.source_node_id == current_node_id && route.signal == signal)
            .ok_or_else(|| ControlRouteSelectionError::RouteNotFound {
                source_node_id: current_node_id.to_owned(),
                signal,
            })?;
        if let Some(proposed_route_id) = proposed_route_id
            && proposed_route_id != route.id
        {
            return Err(ControlRouteSelectionError::ProposalNotSelected {
                proposed_route_id: proposed_route_id.to_owned(),
                selected_route_id: route.id.clone(),
            });
        }
        Ok(SelectedControlRoute {
            route_id: route.id.clone(),
            source_node_id: route.source_node_id.clone(),
            next_node_id: route.destination_node_id.clone(),
            signal,
        })
    }

    /// Resolve one Agent Loop node into the concrete Board scheduling target that
    /// a controller may use to build a Runner request.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the graph is invalid, the node is missing,
    /// or the node is a deterministic, audit, approval, or terminal stage.
    pub fn agent_loop_target(
        &self,
        node_id: &str,
    ) -> Result<AgentLoopExecutionTarget, ControlExecutionTargetError> {
        self.validate()
            .map_err(ControlExecutionTargetError::InvalidGraph)?;
        let node = self
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| ControlExecutionTargetError::NodeNotFound {
                node_id: node_id.to_owned(),
            })?;
        let ControlNodeKind::AgentLoop { agent_profile_id } = &node.kind else {
            return Err(ControlExecutionTargetError::NodeDoesNotExecuteAgent {
                node_id: node_id.to_owned(),
            });
        };
        Ok(AgentLoopExecutionTarget {
            graph_id: self.graph_id.clone(),
            revision_id: self.revision_id.clone(),
            node_id: node.id.clone(),
            agent_profile_id: agent_profile_id.clone(),
        })
    }
}

fn validate_route_authority(
    nodes: &[ControlNode],
    route: &ControlRoute,
) -> Result<(), ControlGraphValidationError> {
    let source = nodes
        .iter()
        .find(|node| node.id == route.source_node_id)
        .ok_or_else(|| ControlGraphValidationError::NodeNotFound {
            node_id: route.source_node_id.clone(),
        })?;
    let valid = match source.kind {
        ControlNodeKind::Approval => matches!(
            route.signal,
            ControlSignal::Approved | ControlSignal::Rejected
        ),
        _ => route.signal != ControlSignal::Approved,
    };
    if !valid {
        return Err(ControlGraphValidationError::InvalidRouteSignal {
            node_id: source.id.clone(),
            signal: route.signal,
        });
    }
    Ok(())
}

/// Named starting point exposed by one Graph revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphEntry {
    pub id: String,
    pub node_id: String,
}

/// One allowed stage in a Control graph.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ControlNode {
    pub id: String,
    pub kind: ControlNodeKind,
}

/// Bounded behavior of one Control node.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlNodeKind {
    AgentLoop { agent_profile_id: String },
    Gate,
    Audit,
    Approval,
    Terminal,
}

/// Named deterministic edge between Control nodes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ControlRoute {
    pub id: String,
    pub source_node_id: String,
    pub destination_node_id: String,
    pub signal: ControlSignal,
}

/// Deterministic result of evaluating one bounded signal at one Control node.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SelectedControlRoute {
    pub route_id: String,
    pub source_node_id: String,
    pub next_node_id: String,
    pub signal: ControlSignal,
}

/// Bounded reason why Safe Autopilot cannot accept a proposed route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlRouteSelectionError {
    InvalidGraph(ControlGraphValidationError),
    NodeNotFound {
        node_id: String,
    },
    RouteNotFound {
        source_node_id: String,
        signal: ControlSignal,
    },
    ProposalNotSelected {
        proposed_route_id: String,
        selected_route_id: String,
    },
}

impl fmt::Display for ControlRouteSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Control route was not selected: {self:?}")
    }
}

impl std::error::Error for ControlRouteSelectionError {}

/// Concrete Agent profile selected by one pinned Agent Loop Control node.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentLoopExecutionTarget {
    pub graph_id: String,
    pub revision_id: String,
    pub node_id: String,
    pub agent_profile_id: String,
}

/// Bounded reason why a current Control node cannot become a Runner target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlExecutionTargetError {
    InvalidGraph(ControlGraphValidationError),
    NodeNotFound { node_id: String },
    NodeDoesNotExecuteAgent { node_id: String },
}

impl fmt::Display for ControlExecutionTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Control node is not an Agent Loop target: {self:?}"
        )
    }
}

impl std::error::Error for ControlExecutionTargetError {}

/// Bounded observation that may select one declared route.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlSignal {
    Succeeded,
    Failed,
    Passed,
    Rejected,
    NeedsApproval,
    Approved,
    BudgetExceeded,
    Manual,
}

/// Fixed grounding that graph optimization cannot rewrite in place.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphAnchor {
    pub id: String,
    pub description: String,
}

/// Exact Graph revision and entry selected as one project's default organization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectGraphBinding {
    pub project_id: String,
    pub graph_id: String,
    pub revision_id: String,
    pub entry_id: String,
}

impl ProjectGraphBinding {
    /// Validate that this binding selects a real entry in the exact revision.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation error when the project identity is invalid,
    /// the revision differs, or the selected entry does not exist.
    pub fn validate_against(
        &self,
        graph: &ControlGraphRevision,
    ) -> Result<(), ControlGraphValidationError> {
        validate_identifier(&self.project_id, "project_id")?;
        if self.graph_id != graph.graph_id || self.revision_id != graph.revision_id {
            return Err(ControlGraphValidationError::RevisionMismatch);
        }
        if !graph.entries.iter().any(|entry| entry.id == self.entry_id) {
            return Err(ControlGraphValidationError::EntryNotFound {
                entry_id: self.entry_id.clone(),
            });
        }
        Ok(())
    }
}

/// Optimistic request to replace one project's complete Graph binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectGraphBindingSaveRequest {
    pub expected: Option<ProjectGraphBinding>,
    pub target: ProjectGraphBinding,
}

/// Durable result of one Project graph binding update.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectGraphBindingSaveReceipt {
    pub previous: Option<ProjectGraphBinding>,
    pub resulting: ProjectGraphBinding,
    pub changed: bool,
}

/// One bounded, explainable transformation proposed for a new Graph revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GraphRewriteOperation {
    ReplaceAgentProfile {
        node_id: String,
        previous_agent_profile_id: String,
        replacement_agent_profile_id: String,
    },
    EditTopology {
        added_nodes: Vec<ControlNode>,
        removed_node_ids: Vec<String>,
        added_routes: Vec<ControlRoute>,
        removed_route_ids: Vec<String>,
    },
}

/// Human-controlled lifecycle of one Graph rewrite proposal.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphRewriteProposalStatus {
    Pending,
    Approved,
    Rejected,
}

impl GraphRewriteProposalStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }
}

/// Stored candidate revision and evidence awaiting an explicit human decision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphRewriteProposal {
    pub proposal_id: String,
    pub project_id: String,
    pub source_binding: ProjectGraphBinding,
    pub candidate_graph: ControlGraphRevision,
    pub operation: GraphRewriteOperation,
    pub rationale: String,
    pub evidence_refs: Vec<String>,
    pub status: GraphRewriteProposalStatus,
}

/// Explicit request to replace one Agent Loop profile in a candidate revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphRewriteProposalRequest {
    pub proposal_id: String,
    pub project_id: String,
    pub expected_source_binding: ProjectGraphBinding,
    pub candidate_revision_id: String,
    pub operation: GraphRewriteOperation,
    pub rationale: String,
    pub evidence_refs: Vec<String>,
}

/// Explicit human decision for one pending Graph rewrite proposal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphRewriteDecision {
    Approve,
    Reject,
}

/// Human intent to decide one project-scoped pending proposal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphRewriteDecisionRequest {
    pub proposal_id: String,
    pub project_id: String,
    pub decision: GraphRewriteDecision,
}

/// Durable result of deciding one Graph rewrite proposal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphRewriteDecisionReceipt {
    pub proposal: GraphRewriteProposal,
    pub previous_binding: ProjectGraphBinding,
    pub resulting_binding: ProjectGraphBinding,
    pub published: bool,
}

/// Board-owned request to evaluate and record one bounded route transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteDecisionRequest {
    pub decision_id: String,
    pub project_id: String,
    pub work_item_id: String,
    pub expected_current_node_id: String,
    pub signal: ControlSignal,
    pub proposed_route_id: Option<String>,
    pub evidence_refs: Vec<String>,
}

/// Immutable accepted route transition for one pinned Work item.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteDecision {
    pub decision_id: String,
    pub project_id: String,
    pub work_item_id: String,
    pub graph_id: String,
    pub revision_id: String,
    pub entry_id: String,
    pub source_node_id: String,
    pub signal: ControlSignal,
    pub proposed_route_id: Option<String>,
    pub route_id: String,
    pub next_node_id: String,
    pub evidence_refs: Vec<String>,
}

/// Current pinned Graph revision and Control node for one Work item.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkItemGraphPosition {
    pub project_id: String,
    pub work_item_id: String,
    pub graph_id: String,
    pub revision_id: String,
    pub entry_id: String,
    pub current_node_id: String,
}

/// Durable outcome of one idempotent Route decision request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteDecisionReceipt {
    pub decision: RouteDecision,
    pub resulting_position: WorkItemGraphPosition,
    pub recorded: bool,
}

/// Bounded reason why a Graph revision is unsafe or ambiguous.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlGraphValidationError {
    InvalidIdentifier {
        field: &'static str,
    },
    MissingEntry,
    MissingNode,
    MissingAnchor,
    DuplicateEntry {
        entry_id: String,
    },
    DuplicateNode {
        node_id: String,
    },
    DuplicateRoute {
        route_id: String,
    },
    DuplicateAnchor {
        anchor_id: String,
    },
    NodeNotFound {
        node_id: String,
    },
    EntryNotFound {
        entry_id: String,
    },
    RevisionMismatch,
    AmbiguousRoute {
        source_node_id: String,
        signal: ControlSignal,
    },
    InvalidRouteSignal {
        node_id: String,
        signal: ControlSignal,
    },
    TerminalHasRoute {
        node_id: String,
    },
    InvalidAnchorDescription,
    InvalidCanvasPosition {
        node_id: String,
    },
}

impl fmt::Display for ControlGraphValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid Control graph: {self:?}")
    }
}

impl std::error::Error for ControlGraphValidationError {}

fn validate_identifier(
    value: &str,
    field: &'static str,
) -> Result<(), ControlGraphValidationError> {
    let length = value.chars().count();
    let mut characters = value.chars();
    let valid_first = characters
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
    if length > 64 || !valid_first || !valid_last || !valid_characters {
        return Err(ControlGraphValidationError::InvalidIdentifier { field });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_control_graph_accepts_a_grounded_deterministic_route() {
        let graph = ControlGraphRevision {
            graph_id: "reviewed".to_owned(),
            revision_id: "v1".to_owned(),
            entries: vec![GraphEntry {
                id: "standard".to_owned(),
                node_id: "implement".to_owned(),
            }],
            nodes: vec![
                ControlNode {
                    id: "implement".to_owned(),
                    kind: ControlNodeKind::AgentLoop {
                        agent_profile_id: "implementer".to_owned(),
                    },
                },
                ControlNode {
                    id: "verify".to_owned(),
                    kind: ControlNodeKind::Audit,
                },
                ControlNode {
                    id: "finish".to_owned(),
                    kind: ControlNodeKind::Terminal,
                },
            ],
            routes: vec![
                ControlRoute {
                    id: "implementation-complete".to_owned(),
                    source_node_id: "implement".to_owned(),
                    destination_node_id: "verify".to_owned(),
                    signal: ControlSignal::Succeeded,
                },
                ControlRoute {
                    id: "verification-passed".to_owned(),
                    source_node_id: "verify".to_owned(),
                    destination_node_id: "finish".to_owned(),
                    signal: ControlSignal::Passed,
                },
            ],
            anchors: vec![GraphAnchor {
                id: "tests-ran".to_owned(),
                description: "Verification is grounded in tests that actually ran".to_owned(),
            }],
        };

        assert_eq!(graph.validate(), Ok(()));
    }

    #[test]
    fn control_graph_uses_the_existing_stable_identifier_vocabulary() {
        let graph = ControlGraphRevision {
            graph_id: "review_graph".to_owned(),
            revision_id: "v1".to_owned(),
            entries: vec![GraphEntry {
                id: "main_entry".to_owned(),
                node_id: "finish_step".to_owned(),
            }],
            nodes: vec![ControlNode {
                id: "finish_step".to_owned(),
                kind: ControlNodeKind::Terminal,
            }],
            routes: Vec::new(),
            anchors: vec![GraphAnchor {
                id: "root_goal".to_owned(),
                description: "The human-selected goal remains fixed".to_owned(),
            }],
        };
        let binding = ProjectGraphBinding {
            project_id: "garage_ai".to_owned(),
            graph_id: "review_graph".to_owned(),
            revision_id: "v1".to_owned(),
            entry_id: "main_entry".to_owned(),
        };

        assert_eq!(graph.validate(), Ok(()));
        assert_eq!(binding.validate_against(&graph), Ok(()));
    }

    #[test]
    fn autopilot_selects_only_the_declared_route_for_the_observed_signal() {
        let graph = ControlGraphRevision {
            graph_id: "direct".to_owned(),
            revision_id: "v1".to_owned(),
            entries: vec![GraphEntry {
                id: "standard".to_owned(),
                node_id: "implement".to_owned(),
            }],
            nodes: vec![
                ControlNode {
                    id: "implement".to_owned(),
                    kind: ControlNodeKind::AgentLoop {
                        agent_profile_id: "implementer".to_owned(),
                    },
                },
                ControlNode {
                    id: "verify".to_owned(),
                    kind: ControlNodeKind::Audit,
                },
            ],
            routes: vec![ControlRoute {
                id: "implementation-complete".to_owned(),
                source_node_id: "implement".to_owned(),
                destination_node_id: "verify".to_owned(),
                signal: ControlSignal::Succeeded,
            }],
            anchors: vec![GraphAnchor {
                id: "tests-ran".to_owned(),
                description: "Verification uses evidence from tests that ran".to_owned(),
            }],
        };

        let selected = graph
            .select_route(
                "implement",
                ControlSignal::Succeeded,
                Some("implementation-complete"),
            )
            .unwrap();

        assert_eq!(selected.route_id, "implementation-complete");
        assert_eq!(selected.next_node_id, "verify");

        let execution_target = graph.agent_loop_target("implement").unwrap();
        assert_eq!(execution_target.node_id, "implement");
        assert_eq!(execution_target.agent_profile_id, "implementer");
    }

    #[test]
    fn approval_node_rejects_a_non_approval_route_signal() {
        let graph = ControlGraphRevision {
            graph_id: "approval-flow".to_owned(),
            revision_id: "v1".to_owned(),
            entries: vec![GraphEntry {
                id: "standard".to_owned(),
                node_id: "approval".to_owned(),
            }],
            nodes: vec![
                ControlNode {
                    id: "approval".to_owned(),
                    kind: ControlNodeKind::Approval,
                },
                ControlNode {
                    id: "finish".to_owned(),
                    kind: ControlNodeKind::Terminal,
                },
            ],
            routes: vec![ControlRoute {
                id: "invalid-pass".to_owned(),
                source_node_id: "approval".to_owned(),
                destination_node_id: "finish".to_owned(),
                signal: ControlSignal::Passed,
            }],
            anchors: vec![GraphAnchor {
                id: "human-authority".to_owned(),
                description: "A human owns the approval decision".to_owned(),
            }],
        };

        assert!(matches!(
            graph.validate(),
            Err(ControlGraphValidationError::InvalidRouteSignal {
                ref node_id,
                signal: ControlSignal::Passed,
            }) if node_id == "approval"
        ));
    }
}
