use std::collections::{HashSet, VecDeque};

use gareji_board_domain::{
    ControlGraphRevision, ControlNode, ControlNodeKind, ControlRoute, ControlSignal,
    GraphRewriteOperation,
};
use thiserror::Error;

/// In-memory editor for one immutable Control graph revision.
///
/// The draft keeps entries and anchors fixed, validates every structural edit,
/// and emits one bounded topology operation for the approval workflow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphDraft {
    source: ControlGraphRevision,
    graph: ControlGraphRevision,
    history: Vec<ControlGraphRevision>,
}

impl GraphDraft {
    /// Start a draft from one valid immutable revision.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the source revision is invalid.
    pub fn new(source: ControlGraphRevision) -> Result<Self, GraphDraftError> {
        source
            .validate()
            .map_err(|_| GraphDraftError::InvalidGraph)?;
        Ok(Self {
            graph: source.clone(),
            source,
            history: Vec::new(),
        })
    }

    /// Read the current preview without exposing mutable graph state.
    #[must_use]
    pub const fn graph(&self) -> &ControlGraphRevision {
        &self.graph
    }

    /// Add one Agent Loop node to the draft.
    ///
    /// # Errors
    ///
    /// Returns a bounded error for invalid or duplicate identities.
    pub fn add_agent_loop(
        &mut self,
        node_id: String,
        agent_profile_id: String,
    ) -> Result<(), GraphDraftError> {
        self.add_node(node_id, ControlNodeKind::AgentLoop { agent_profile_id })
    }

    /// Add one supported Control node kind to the draft.
    ///
    /// # Errors
    ///
    /// Returns a bounded error for invalid or duplicate identities.
    pub fn add_node(
        &mut self,
        node_id: String,
        kind: ControlNodeKind,
    ) -> Result<(), GraphDraftError> {
        self.apply_candidate_change(|graph| graph.nodes.push(ControlNode { id: node_id, kind }))
    }

    /// Remove one non-entry node and all of its incident routes.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the node is missing or owns an entry.
    pub fn remove_node(&mut self, node_id: &str) -> Result<(), GraphDraftError> {
        if self
            .graph
            .entries
            .iter()
            .any(|entry| entry.node_id == node_id)
        {
            return Err(GraphDraftError::EntryNodeCannotBeRemoved);
        }
        if !self.graph.nodes.iter().any(|node| node.id == node_id) {
            return Err(GraphDraftError::NodeNotFound);
        }
        self.apply_candidate_change(|graph| {
            graph.nodes.retain(|node| node.id != node_id);
            graph.routes.retain(|route| {
                route.source_node_id != node_id && route.destination_node_id != node_id
            });
        })
    }

    /// Connect two existing nodes with one bounded signal.
    ///
    /// # Errors
    ///
    /// Returns a bounded error for missing nodes, duplicate identities,
    /// ambiguous source signals, invalid authority, or terminal sources.
    pub fn connect(
        &mut self,
        route_id: String,
        source_node_id: String,
        destination_node_id: String,
        signal: ControlSignal,
    ) -> Result<(), GraphDraftError> {
        self.apply_candidate_change(|graph| {
            graph.routes.push(ControlRoute {
                id: route_id,
                source_node_id,
                destination_node_id,
                signal,
            });
        })
    }

    /// Remove one declared route from the draft.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the route does not exist.
    pub fn remove_route(&mut self, route_id: &str) -> Result<(), GraphDraftError> {
        if !self.graph.routes.iter().any(|route| route.id == route_id) {
            return Err(GraphDraftError::RouteNotFound);
        }
        self.apply_candidate_change(|graph| graph.routes.retain(|route| route.id != route_id))
    }

    /// Restore the immediately preceding valid draft state.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the draft has no earlier state.
    pub fn undo(&mut self) -> Result<(), GraphDraftError> {
        let previous = self.history.pop().ok_or(GraphDraftError::NothingToUndo)?;
        self.graph = previous;
        Ok(())
    }

    /// Restore the immutable source revision while retaining one undo step.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the draft already matches its source.
    pub fn reset(&mut self) -> Result<(), GraphDraftError> {
        if !self.has_changes() {
            return Err(GraphDraftError::NoChange);
        }
        self.history.push(self.graph.clone());
        self.graph.clone_from(&self.source);
        Ok(())
    }

    /// Return whether an earlier valid draft state can be restored.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    /// Apply one stored rewrite operation to the draft.
    ///
    /// This is the single reconstruction path used by controllers before a
    /// candidate is persisted, so callers cannot submit a hidden full graph.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the operation does not match the source.
    pub fn apply_operation(
        &mut self,
        operation: &GraphRewriteOperation,
    ) -> Result<(), GraphDraftError> {
        match operation {
            GraphRewriteOperation::ReplaceAgentProfile {
                node_id,
                previous_agent_profile_id,
                replacement_agent_profile_id,
            } => self.apply_candidate_change(|graph| {
                let Some(node) = graph.nodes.iter_mut().find(|node| node.id == *node_id) else {
                    return;
                };
                let ControlNodeKind::AgentLoop { agent_profile_id } = &mut node.kind else {
                    return;
                };
                if agent_profile_id == previous_agent_profile_id {
                    agent_profile_id.clone_from(replacement_agent_profile_id);
                }
            }),
            GraphRewriteOperation::EditTopology {
                added_nodes,
                removed_node_ids,
                added_routes,
                removed_route_ids,
            } => self.apply_candidate_change(|graph| {
                graph
                    .routes
                    .retain(|route| !removed_route_ids.contains(&route.id));
                graph
                    .nodes
                    .retain(|node| !removed_node_ids.contains(&node.id));
                graph.nodes.extend(added_nodes.iter().cloned());
                graph.routes.extend(added_routes.iter().cloned());
            }),
        }
    }

    /// Materialize the current draft under a new immutable revision identity.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when validation or reachability fails.
    pub fn revision(
        &self,
        candidate_revision_id: &str,
    ) -> Result<ControlGraphRevision, GraphDraftError> {
        let mut candidate = self.graph.clone();
        candidate_revision_id.clone_into(&mut candidate.revision_id);
        candidate
            .validate()
            .map_err(|_| GraphDraftError::InvalidGraph)?;
        if !all_nodes_are_reachable(&candidate) {
            return Err(GraphDraftError::UnreachableNode);
        }
        Ok(candidate)
    }

    /// Build the complete candidate revision for an approval request.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when nothing changed, an existing definition was
    /// mutated in place, or a node is unreachable from every fixed entry.
    pub fn candidate(
        &self,
        candidate_revision_id: &str,
    ) -> Result<(ControlGraphRevision, GraphRewriteOperation), GraphDraftError> {
        let operation = self.operation()?;
        let candidate = self.revision(candidate_revision_id)?;
        Ok((candidate, operation))
    }

    /// Return whether the draft differs from its immutable source.
    #[must_use]
    pub fn has_changes(&self) -> bool {
        self.source.nodes != self.graph.nodes || self.source.routes != self.graph.routes
    }

    fn operation(&self) -> Result<GraphRewriteOperation, GraphDraftError> {
        let source_node_ids = self
            .source
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<HashSet<_>>();
        let draft_node_ids = self
            .graph
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<HashSet<_>>();
        let source_route_ids = self
            .source
            .routes
            .iter()
            .map(|route| route.id.as_str())
            .collect::<HashSet<_>>();
        let draft_route_ids = self
            .graph
            .routes
            .iter()
            .map(|route| route.id.as_str())
            .collect::<HashSet<_>>();

        if self.source.nodes.iter().any(|source| {
            self.graph
                .nodes
                .iter()
                .find(|node| node.id == source.id)
                .is_some_and(|draft| draft != source)
        }) {
            return Err(GraphDraftError::ExistingNodeChanged);
        }
        if self.source.routes.iter().any(|source| {
            self.graph
                .routes
                .iter()
                .find(|route| route.id == source.id)
                .is_some_and(|draft| draft != source)
        }) {
            return Err(GraphDraftError::ExistingRouteChanged);
        }

        let added_nodes = self
            .graph
            .nodes
            .iter()
            .filter(|node| !source_node_ids.contains(node.id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let removed_node_ids = self
            .source
            .nodes
            .iter()
            .filter(|node| !draft_node_ids.contains(node.id.as_str()))
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        let added_routes = self
            .graph
            .routes
            .iter()
            .filter(|route| !source_route_ids.contains(route.id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let removed_route_ids = self
            .source
            .routes
            .iter()
            .filter(|route| !draft_route_ids.contains(route.id.as_str()))
            .map(|route| route.id.clone())
            .collect::<Vec<_>>();

        if added_nodes.is_empty()
            && removed_node_ids.is_empty()
            && added_routes.is_empty()
            && removed_route_ids.is_empty()
        {
            return Err(GraphDraftError::NoChange);
        }
        Ok(GraphRewriteOperation::EditTopology {
            added_nodes,
            removed_node_ids,
            added_routes,
            removed_route_ids,
        })
    }

    fn apply_candidate_change(
        &mut self,
        change: impl FnOnce(&mut ControlGraphRevision),
    ) -> Result<(), GraphDraftError> {
        let mut candidate = self.graph.clone();
        change(&mut candidate);
        candidate
            .validate()
            .map_err(|_| GraphDraftError::InvalidGraph)?;
        if candidate == self.graph {
            return Err(GraphDraftError::NoChange);
        }
        self.history.push(self.graph.clone());
        self.graph = candidate;
        Ok(())
    }
}

fn all_nodes_are_reachable(graph: &ControlGraphRevision) -> bool {
    let mut reachable = graph
        .entries
        .iter()
        .map(|entry| entry.node_id.as_str())
        .collect::<HashSet<_>>();
    let mut queue = reachable.iter().copied().collect::<VecDeque<_>>();
    while let Some(source) = queue.pop_front() {
        for destination in graph
            .routes
            .iter()
            .filter(|route| route.source_node_id == source)
            .map(|route| route.destination_node_id.as_str())
        {
            if reachable.insert(destination) {
                queue.push_back(destination);
            }
        }
    }
    reachable.len() == graph.nodes.len()
}

/// Bounded reason why a visual Graph draft edit could not be applied.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum GraphDraftError {
    #[error("the Graph draft is invalid")]
    InvalidGraph,
    #[error("the selected Control node was not found")]
    NodeNotFound,
    #[error("a Graph entry node cannot be removed")]
    EntryNodeCannotBeRemoved,
    #[error("the selected Control route was not found")]
    RouteNotFound,
    #[error("the Graph draft has no changes")]
    NoChange,
    #[error("the Graph draft has no earlier state")]
    NothingToUndo,
    #[error("an existing Control node cannot be mutated through a topology edit")]
    ExistingNodeChanged,
    #[error("an existing Control route cannot be mutated through a topology edit")]
    ExistingRouteChanged,
    #[error("every Control node must be reachable from a Graph entry")]
    UnreachableNode,
}

#[cfg(test)]
mod tests {
    use gareji_board_domain::{
        ControlGraphRevision, ControlNode, ControlNodeKind, ControlRoute, ControlSignal,
        GraphAnchor, GraphEntry, GraphRewriteOperation,
    };

    use super::{GraphDraft, GraphDraftError};

    #[test]
    fn opens_every_builtin_graph_revision_for_visual_editing() {
        let mut store = gareji_board_store::SqliteBoardStore::open_in_memory().unwrap();
        store.ensure_builtin_control_graphs().unwrap();

        for graph in store.load_control_graph_revisions().unwrap() {
            GraphDraft::new(graph).unwrap();
        }
    }

    #[test]
    fn builds_one_connected_topology_operation_without_mutating_the_source() {
        let source = source_graph();
        let mut draft = GraphDraft::new(source.clone()).unwrap();
        draft.remove_route("implementation-complete").unwrap();
        draft
            .add_agent_loop("specialist".to_owned(), "reviewer".to_owned())
            .unwrap();
        draft
            .connect(
                "implementation-specialist".to_owned(),
                "implement".to_owned(),
                "specialist".to_owned(),
                ControlSignal::Succeeded,
            )
            .unwrap();
        draft
            .connect(
                "specialist-review".to_owned(),
                "specialist".to_owned(),
                "review".to_owned(),
                ControlSignal::Succeeded,
            )
            .unwrap();

        let (candidate, operation) = draft.candidate("v2").unwrap();

        assert_eq!(source.revision_id, "v1");
        assert_eq!(candidate.revision_id, "v2");
        assert!(candidate.nodes.iter().any(|node| node.id == "specialist"));
        let GraphRewriteOperation::EditTopology {
            added_nodes,
            removed_route_ids,
            added_routes,
            ..
        } = operation
        else {
            panic!("expected topology edit")
        };
        assert_eq!(added_nodes[0].id, "specialist");
        assert_eq!(removed_route_ids, vec!["implementation-complete"]);
        assert_eq!(added_routes.len(), 2);
    }

    #[test]
    fn entry_nodes_and_unreachable_nodes_fail_closed() {
        let mut draft = GraphDraft::new(source_graph()).unwrap();
        assert_eq!(
            draft.remove_node("implement"),
            Err(GraphDraftError::EntryNodeCannotBeRemoved)
        );
        draft
            .add_agent_loop("orphan".to_owned(), "reviewer".to_owned())
            .unwrap();
        assert_eq!(draft.candidate("v2"), Err(GraphDraftError::UnreachableNode));
    }

    #[test]
    fn supports_every_visual_node_kind_and_reversible_draft_history() {
        let mut draft = GraphDraft::new(source_graph()).unwrap();
        for (node_id, kind) in [
            (
                "agent",
                ControlNodeKind::AgentLoop {
                    agent_profile_id: "reviewer".to_owned(),
                },
            ),
            ("gate", ControlNodeKind::Gate),
            ("approval", ControlNodeKind::Approval),
            ("audit", ControlNodeKind::Audit),
            ("terminal", ControlNodeKind::Terminal),
        ] {
            draft.add_node(node_id.to_owned(), kind).unwrap();
        }
        assert!(draft.can_undo());
        assert_eq!(draft.graph().nodes.len(), 8);

        draft.undo().unwrap();
        assert!(!draft.graph().nodes.iter().any(|node| node.id == "terminal"));
        draft.reset().unwrap();
        assert!(!draft.has_changes());
        assert_eq!(draft.graph().nodes.len(), 3);

        draft.undo().unwrap();
        assert!(draft.graph().nodes.iter().any(|node| node.id == "audit"));
    }

    #[test]
    fn undo_and_reset_fail_closed_without_a_prior_change() {
        let mut draft = GraphDraft::new(source_graph()).unwrap();
        assert_eq!(draft.undo(), Err(GraphDraftError::NothingToUndo));
        assert_eq!(draft.reset(), Err(GraphDraftError::NoChange));
    }

    fn source_graph() -> ControlGraphRevision {
        ControlGraphRevision {
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
                    id: "review".to_owned(),
                    kind: ControlNodeKind::AgentLoop {
                        agent_profile_id: "reviewer".to_owned(),
                    },
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
                    destination_node_id: "review".to_owned(),
                    signal: ControlSignal::Succeeded,
                },
                ControlRoute {
                    id: "review-passed".to_owned(),
                    source_node_id: "review".to_owned(),
                    destination_node_id: "finish".to_owned(),
                    signal: ControlSignal::Passed,
                },
            ],
            anchors: vec![GraphAnchor {
                id: "verified".to_owned(),
                description: "Completion remains independently verified".to_owned(),
            }],
        }
    }
}
