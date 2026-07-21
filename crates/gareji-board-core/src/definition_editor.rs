use std::path::Path;

use gareji_board_domain::{
    BlueprintLink, BlueprintNode, ControlGraphRevision, ControlNode, ControlRoute,
    OrchestrationBlueprintRevision,
};
use gareji_board_store::{SqliteBoardStore, StoreError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{BlueprintDraft, BlueprintDraftError, GraphDraft, GraphDraftError};

/// One atomic set of Control Graph edits that publishes a new immutable revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ControlGraphEditPlan {
    pub graph_id: String,
    pub source_revision_id: String,
    pub new_revision_id: String,
    pub operations: Vec<ControlGraphEdit>,
}

/// One bounded Control Graph topology edit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum ControlGraphEdit {
    AddNode { node: ControlNode },
    RemoveNode { node_id: String },
    Connect { route: ControlRoute },
    RemoveRoute { route_id: String },
}

/// One atomic set of Blueprint edits that publishes a new immutable revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BlueprintEditPlan {
    pub blueprint_id: String,
    pub source_revision_id: String,
    pub new_revision_id: String,
    pub operations: Vec<BlueprintEdit>,
}

/// One bounded Orchestration Blueprint topology edit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum BlueprintEdit {
    AddNode { node: BlueprintNode },
    RemoveNode { node_id: String },
    Connect { link: BlueprintLink },
    RemoveLink { link_id: String },
}

/// Deep Module for inspecting and atomically publishing Board definitions.
///
/// Callers submit a complete edit plan. The implementation keeps temporary,
/// incomplete topology inside a draft and exposes only a validated immutable
/// revision to storage.
pub struct DefinitionEditor {
    store: SqliteBoardStore,
}

impl DefinitionEditor {
    /// Open Board-owned definitions from one local database.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the database cannot be opened.
    pub fn open(path: &Path) -> Result<Self, DefinitionEditorError> {
        Ok(Self::new(SqliteBoardStore::open(path)?))
    }

    /// Compose the Module with a local-substitutable store.
    #[must_use]
    pub const fn new(store: SqliteBoardStore) -> Self {
        Self { store }
    }

    /// Load every immutable Control Graph revision in stable identity order.
    ///
    /// # Errors
    ///
    /// Returns a storage or corrupt-state error.
    pub fn control_graphs(&self) -> Result<Vec<ControlGraphRevision>, DefinitionEditorError> {
        Ok(self.store.load_control_graph_revisions()?)
    }

    /// Load one exact immutable Control Graph revision.
    ///
    /// # Errors
    ///
    /// Returns a bounded not-found or storage error.
    pub fn control_graph(
        &self,
        graph_id: &str,
        revision_id: &str,
    ) -> Result<ControlGraphRevision, DefinitionEditorError> {
        self.control_graphs()?
            .into_iter()
            .find(|graph| graph.graph_id == graph_id && graph.revision_id == revision_id)
            .ok_or_else(|| DefinitionEditorError::ControlGraphNotFound {
                graph_id: graph_id.to_owned(),
                revision_id: revision_id.to_owned(),
            })
    }

    /// Apply all edits in memory and publish one valid Control Graph revision.
    ///
    /// # Errors
    ///
    /// Returns a bounded draft, identity, or storage error without a partial write.
    pub fn apply_control_graph(
        &mut self,
        plan: &ControlGraphEditPlan,
    ) -> Result<ControlGraphRevision, DefinitionEditorError> {
        validate_plan_identity(
            &plan.source_revision_id,
            &plan.new_revision_id,
            plan.operations.is_empty(),
        )?;
        let source = self.control_graph(&plan.graph_id, &plan.source_revision_id)?;
        let mut draft = GraphDraft::new(source)?;
        for operation in &plan.operations {
            match operation {
                ControlGraphEdit::AddNode { node } => {
                    draft.add_node(node.id.clone(), node.kind.clone())?;
                }
                ControlGraphEdit::RemoveNode { node_id } => draft.remove_node(node_id)?,
                ControlGraphEdit::Connect { route } => draft.connect(
                    route.id.clone(),
                    route.source_node_id.clone(),
                    route.destination_node_id.clone(),
                    route.signal,
                )?,
                ControlGraphEdit::RemoveRoute { route_id } => draft.remove_route(route_id)?,
            }
        }
        let revision = draft.revision(&plan.new_revision_id)?;
        self.store.save_control_graph_revision(&revision)?;
        Ok(revision)
    }

    /// Load every immutable Orchestration Blueprint revision in stable order.
    ///
    /// # Errors
    ///
    /// Returns a storage or corrupt-state error.
    pub fn blueprints(&self) -> Result<Vec<OrchestrationBlueprintRevision>, DefinitionEditorError> {
        Ok(self.store.load_orchestration_blueprint_revisions()?)
    }

    /// Load one exact immutable Orchestration Blueprint revision.
    ///
    /// # Errors
    ///
    /// Returns a bounded not-found or storage error.
    pub fn blueprint(
        &self,
        blueprint_id: &str,
        revision_id: &str,
    ) -> Result<OrchestrationBlueprintRevision, DefinitionEditorError> {
        self.blueprints()?
            .into_iter()
            .find(|blueprint| {
                blueprint.blueprint_id == blueprint_id && blueprint.revision_id == revision_id
            })
            .ok_or_else(|| DefinitionEditorError::BlueprintNotFound {
                blueprint_id: blueprint_id.to_owned(),
                revision_id: revision_id.to_owned(),
            })
    }

    /// Apply all edits in memory and publish one valid Blueprint revision.
    ///
    /// # Errors
    ///
    /// Returns a bounded draft, identity, or storage error without a partial write.
    pub fn apply_blueprint(
        &mut self,
        plan: &BlueprintEditPlan,
    ) -> Result<OrchestrationBlueprintRevision, DefinitionEditorError> {
        validate_plan_identity(
            &plan.source_revision_id,
            &plan.new_revision_id,
            plan.operations.is_empty(),
        )?;
        let source = self.blueprint(&plan.blueprint_id, &plan.source_revision_id)?;
        let mut draft = BlueprintDraft::new(source)?;
        for operation in &plan.operations {
            match operation {
                BlueprintEdit::AddNode { node } => draft.add_node(node.clone())?,
                BlueprintEdit::RemoveNode { node_id } => draft.remove_node(node_id)?,
                BlueprintEdit::Connect { link } => draft.connect(
                    link.id.clone(),
                    link.source_node_id.clone(),
                    link.destination_node_id.clone(),
                    link.kind,
                )?,
                BlueprintEdit::RemoveLink { link_id } => draft.remove_link(link_id)?,
            }
        }
        let revision = draft.revision(&plan.new_revision_id)?;
        self.store
            .save_orchestration_blueprint_revision(&revision)?;
        Ok(revision)
    }
}

fn validate_plan_identity(
    source_revision_id: &str,
    new_revision_id: &str,
    is_empty: bool,
) -> Result<(), DefinitionEditorError> {
    if is_empty {
        return Err(DefinitionEditorError::EmptyPlan);
    }
    if source_revision_id == new_revision_id {
        return Err(DefinitionEditorError::RevisionIdentityUnchanged);
    }
    Ok(())
}

/// Bounded failure returned while inspecting or publishing definitions.
#[derive(Debug, Error)]
pub enum DefinitionEditorError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    GraphDraft(#[from] GraphDraftError),
    #[error(transparent)]
    BlueprintDraft(#[from] BlueprintDraftError),
    #[error("Control Graph {graph_id}/{revision_id} was not found")]
    ControlGraphNotFound {
        graph_id: String,
        revision_id: String,
    },
    #[error("Blueprint {blueprint_id}/{revision_id} was not found")]
    BlueprintNotFound {
        blueprint_id: String,
        revision_id: String,
    },
    #[error("definition edit plan contains no operations")]
    EmptyPlan,
    #[error("new revision identity must differ from the source revision")]
    RevisionIdentityUnchanged,
}

#[cfg(test)]
mod tests {
    use gareji_board_domain::{
        BlueprintLinkKind, BlueprintNodeKind, BlueprintScope, ControlNodeKind, ControlSignal,
        GraphAnchor, GraphEntry, NoteSocketKind,
    };

    use super::*;

    #[test]
    fn control_graph_plan_publishes_all_topology_edits_atomically() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.save_control_graph_revision(&source_graph()).unwrap();
        let mut editor = DefinitionEditor::new(store);
        let plan = ControlGraphEditPlan {
            graph_id: "delivery".to_owned(),
            source_revision_id: "v1".to_owned(),
            new_revision_id: "v2".to_owned(),
            operations: vec![
                ControlGraphEdit::RemoveRoute {
                    route_id: "implement-finish".to_owned(),
                },
                ControlGraphEdit::AddNode {
                    node: ControlNode {
                        id: "audit".to_owned(),
                        kind: ControlNodeKind::Audit,
                    },
                },
                ControlGraphEdit::Connect {
                    route: ControlRoute {
                        id: "implement-audit".to_owned(),
                        source_node_id: "implement".to_owned(),
                        destination_node_id: "audit".to_owned(),
                        signal: ControlSignal::Succeeded,
                    },
                },
                ControlGraphEdit::Connect {
                    route: ControlRoute {
                        id: "audit-finish".to_owned(),
                        source_node_id: "audit".to_owned(),
                        destination_node_id: "finish".to_owned(),
                        signal: ControlSignal::Passed,
                    },
                },
            ],
        };

        let revision = editor.apply_control_graph(&plan).unwrap();
        assert_eq!(revision.revision_id, "v2");
        assert_eq!(revision.nodes.len(), 3);
        assert_eq!(revision.routes.len(), 2);
        assert_eq!(
            editor.control_graph("delivery", "v1").unwrap(),
            source_graph()
        );
    }

    #[test]
    fn blueprint_plan_can_add_and_connect_one_portable_stage() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store
            .save_orchestration_blueprint_revision(&source_blueprint())
            .unwrap();
        let mut editor = DefinitionEditor::new(store);
        let plan = BlueprintEditPlan {
            blueprint_id: "evidence-first".to_owned(),
            source_revision_id: "v1".to_owned(),
            new_revision_id: "v2".to_owned(),
            operations: vec![
                BlueprintEdit::RemoveLink {
                    link_id: "approach-finish".to_owned(),
                },
                BlueprintEdit::AddNode {
                    node: BlueprintNode {
                        id: "gate".to_owned(),
                        kind: BlueprintNodeKind::Gate,
                        inputs: Vec::new(),
                        outputs: Vec::new(),
                    },
                },
                BlueprintEdit::Connect {
                    link: BlueprintLink {
                        id: "approach-gate".to_owned(),
                        source_node_id: "approach".to_owned(),
                        destination_node_id: "gate".to_owned(),
                        kind: BlueprintLinkKind::Flow {
                            signal: ControlSignal::Succeeded,
                        },
                    },
                },
                BlueprintEdit::Connect {
                    link: BlueprintLink {
                        id: "gate-finish".to_owned(),
                        source_node_id: "gate".to_owned(),
                        destination_node_id: "finish".to_owned(),
                        kind: BlueprintLinkKind::Flow {
                            signal: ControlSignal::Passed,
                        },
                    },
                },
            ],
        };

        let revision = editor.apply_blueprint(&plan).unwrap();
        assert_eq!(revision.revision_id, "v2");
        assert_eq!(revision.nodes.len(), 3);
        assert_eq!(revision.links.len(), 2);
        assert_eq!(
            editor.blueprint("evidence-first", "v1").unwrap(),
            source_blueprint()
        );
    }

    fn source_graph() -> ControlGraphRevision {
        ControlGraphRevision {
            graph_id: "delivery".to_owned(),
            revision_id: "v1".to_owned(),
            entries: vec![GraphEntry {
                id: "default".to_owned(),
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
                    id: "finish".to_owned(),
                    kind: ControlNodeKind::Terminal,
                },
            ],
            routes: vec![ControlRoute {
                id: "implement-finish".to_owned(),
                source_node_id: "implement".to_owned(),
                destination_node_id: "finish".to_owned(),
                signal: ControlSignal::Succeeded,
            }],
            anchors: vec![GraphAnchor {
                id: "scope".to_owned(),
                description: "Keep delivery bounded.".to_owned(),
            }],
        }
    }

    fn source_blueprint() -> OrchestrationBlueprintRevision {
        OrchestrationBlueprintRevision {
            blueprint_id: "evidence-first".to_owned(),
            revision_id: "v1".to_owned(),
            name: "Evidence first".to_owned(),
            scope: BlueprintScope::Project,
            entry_node_id: "approach".to_owned(),
            nodes: vec![
                BlueprintNode {
                    id: "approach".to_owned(),
                    kind: BlueprintNodeKind::Approach {
                        approach_id: "evidence".to_owned(),
                    },
                    inputs: vec![NoteSocketKind::Context],
                    outputs: vec![NoteSocketKind::Evidence],
                },
                BlueprintNode {
                    id: "finish".to_owned(),
                    kind: BlueprintNodeKind::Terminal,
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                },
            ],
            links: vec![BlueprintLink {
                id: "approach-finish".to_owned(),
                source_node_id: "approach".to_owned(),
                destination_node_id: "finish".to_owned(),
                kind: BlueprintLinkKind::Flow {
                    signal: ControlSignal::Succeeded,
                },
            }],
        }
    }
}
