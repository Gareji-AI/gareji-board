use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{ControlSignal, ExecutionWorkspaceConnection};

/// Immutable, project-independent organization that can be applied repeatedly.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrchestrationBlueprintRevision {
    pub blueprint_id: String,
    pub revision_id: String,
    pub name: String,
    pub scope: BlueprintScope,
    pub entry_node_id: String,
    pub nodes: Vec<BlueprintNode>,
    pub links: Vec<BlueprintLink>,
}

impl OrchestrationBlueprintRevision {
    /// Validate portable identity, topology, typed sockets, and authority.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when this revision cannot be safely applied.
    pub fn validate(&self) -> Result<(), BlueprintValidationError> {
        self.validate_editable_structure()?;
        let has_terminal = self
            .nodes
            .iter()
            .any(|node| matches!(node.kind, BlueprintNodeKind::Terminal));
        if !has_terminal {
            return Err(BlueprintValidationError::MissingTerminal);
        }
        let has_scope_work = self.nodes.iter().any(|node| match self.scope {
            BlueprintScope::Project => matches!(node.kind, BlueprintNodeKind::Approach { .. }),
            BlueprintScope::Portfolio => {
                matches!(node.kind, BlueprintNodeKind::ProjectSelector { .. })
            }
        });
        if !has_scope_work {
            return Err(BlueprintValidationError::MissingScopeWork { scope: self.scope });
        }
        self.validate_reachability()
    }

    /// Validate identities, node contracts, links, sockets, and authority while editing.
    ///
    /// Unlike [`Self::validate`], this permits temporarily unreachable nodes and a
    /// temporarily missing terminal or scope-work node. Immutable revisions must
    /// still pass the complete validation before they can be stored.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when an individual draft edit violates the contract.
    pub fn validate_editable_structure(&self) -> Result<(), BlueprintValidationError> {
        validate_identifier(&self.blueprint_id, "blueprint_id")?;
        validate_identifier(&self.revision_id, "revision_id")?;
        validate_identifier(&self.entry_node_id, "entry_node_id")?;
        validate_name(&self.name)?;
        if self.nodes.is_empty() {
            return Err(BlueprintValidationError::MissingNode);
        }

        let node_ids = self.validate_nodes()?;
        if !node_ids.contains(self.entry_node_id.as_str()) {
            return Err(BlueprintValidationError::NodeNotFound {
                node_id: self.entry_node_id.clone(),
            });
        }
        self.validate_links(&node_ids)?;
        Ok(())
    }

    /// Resolve the declared entry after validating the complete revision.
    ///
    /// # Errors
    ///
    /// Returns the same bounded errors as [`Self::validate`].
    pub fn entry_node(&self) -> Result<&BlueprintNode, BlueprintValidationError> {
        self.validate()?;
        self.nodes
            .iter()
            .find(|node| node.id == self.entry_node_id)
            .ok_or_else(|| BlueprintValidationError::NodeNotFound {
                node_id: self.entry_node_id.clone(),
            })
    }

    /// Return every Approach identity referenced by this exact revision.
    #[must_use]
    pub fn approach_ids(&self) -> Vec<&str> {
        self.nodes
            .iter()
            .filter_map(|node| match &node.kind {
                BlueprintNodeKind::Approach { approach_id } => Some(approach_id.as_str()),
                _ => None,
            })
            .collect()
    }

    fn validate_nodes(&self) -> Result<HashSet<&str>, BlueprintValidationError> {
        let mut node_ids = HashSet::with_capacity(self.nodes.len());
        for node in &self.nodes {
            validate_identifier(&node.id, "node_id")?;
            if !node_ids.insert(node.id.as_str()) {
                return Err(BlueprintValidationError::DuplicateNode {
                    node_id: node.id.clone(),
                });
            }
            validate_node_kind(self, node)?;
            validate_sockets(node)?;
        }
        Ok(node_ids)
    }

    fn validate_links(&self, node_ids: &HashSet<&str>) -> Result<(), BlueprintValidationError> {
        let mut link_ids = HashSet::with_capacity(self.links.len());
        let mut flow_keys = HashSet::new();
        let mut data_keys = HashSet::new();
        for link in &self.links {
            validate_identifier(&link.id, "link_id")?;
            if !link_ids.insert(link.id.as_str()) {
                return Err(BlueprintValidationError::DuplicateLink {
                    link_id: link.id.clone(),
                });
            }
            for node_id in [&link.source_node_id, &link.destination_node_id] {
                if !node_ids.contains(node_id.as_str()) {
                    return Err(BlueprintValidationError::NodeNotFound {
                        node_id: node_id.clone(),
                    });
                }
            }
            let source = find_node(&self.nodes, &link.source_node_id)?;
            let destination = find_node(&self.nodes, &link.destination_node_id)?;
            match link.kind {
                BlueprintLinkKind::Flow { signal } => {
                    if !flow_keys.insert((source.id.as_str(), signal)) {
                        return Err(BlueprintValidationError::AmbiguousFlow {
                            source_node_id: source.id.clone(),
                            signal,
                        });
                    }
                    validate_flow_authority(source, signal)?;
                }
                BlueprintLinkKind::Data { socket } => {
                    if !data_keys.insert((source.id.as_str(), destination.id.as_str(), socket)) {
                        return Err(BlueprintValidationError::DuplicateDataLink {
                            source_node_id: source.id.clone(),
                            destination_node_id: destination.id.clone(),
                            socket,
                        });
                    }
                    if !source.outputs.contains(&socket) || !destination.inputs.contains(&socket) {
                        return Err(BlueprintValidationError::IncompatibleSocket {
                            source_node_id: source.id.clone(),
                            destination_node_id: destination.id.clone(),
                            socket,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_reachability(&self) -> Result<(), BlueprintValidationError> {
        let mut reachable = HashSet::from([self.entry_node_id.as_str()]);
        let mut queue = VecDeque::from([self.entry_node_id.as_str()]);
        while let Some(source_node_id) = queue.pop_front() {
            for destination_node_id in self
                .links
                .iter()
                .filter(|link| {
                    link.source_node_id == source_node_id
                        && matches!(link.kind, BlueprintLinkKind::Flow { .. })
                })
                .map(|link| link.destination_node_id.as_str())
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
            return Err(BlueprintValidationError::UnreachableNode {
                node_id: node.id.clone(),
            });
        }
        Ok(())
    }
}

/// Execution context in which a Blueprint may be applied.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintScope {
    Project,
    Portfolio,
}

/// One stage and its typed knowledge/data sockets.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BlueprintNode {
    pub id: String,
    pub kind: BlueprintNodeKind,
    pub inputs: Vec<NoteSocketKind>,
    pub outputs: Vec<NoteSocketKind>,
}

/// Portable behavior declared by a Blueprint node.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BlueprintNodeKind {
    Approach {
        approach_id: String,
    },
    Gate,
    Audit,
    Approval,
    ProjectSelector {
        target_blueprint_id: String,
        target_revision_id: String,
        target_entry_id: String,
    },
    PostAction {
        action: BlueprintPostAction,
    },
    Terminal,
}

/// Bounded portfolio-level action that does not directly start a Runner.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintPostAction {
    RecordSummary,
}

/// One typed control-flow or data connection between Blueprint nodes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BlueprintLink {
    pub id: String,
    pub source_node_id: String,
    pub destination_node_id: String,
    pub kind: BlueprintLinkKind,
}

/// Meaning carried by one Blueprint connection.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BlueprintLinkKind {
    Flow { signal: ControlSignal },
    Data { socket: NoteSocketKind },
}

/// Typed value that may enter or leave an Approach Note node.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteSocketKind {
    Context,
    WorkItem,
    Evidence,
    Artifact,
    Signal,
    Approval,
}

/// Risk declared by an Approach Note before it can be applied.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApproachRisk {
    Low,
    Moderate,
    High,
    Critical,
}

/// Validated metadata read from one local Markdown Approach Note.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApproachNoteManifest {
    pub approach_id: String,
    pub title: String,
    pub absolute_path: String,
    pub fingerprint: String,
    pub required_capabilities: Vec<String>,
    pub inputs: Vec<NoteSocketKind>,
    pub outputs: Vec<NoteSocketKind>,
    pub risk: ApproachRisk,
}

impl ApproachNoteManifest {
    /// Validate portable identity and the pinned local note reference.
    ///
    /// # Errors
    ///
    /// Returns a bounded error for malformed note metadata.
    pub fn validate(&self) -> Result<(), ApproachNoteValidationError> {
        validate_identifier(&self.approach_id, "approach_id")
            .map_err(ApproachNoteValidationError::InvalidIdentity)?;
        validate_name(&self.title).map_err(|_| ApproachNoteValidationError::InvalidTitle)?;
        if !Path::new(&self.absolute_path).is_absolute() {
            return Err(ApproachNoteValidationError::PathNotAbsolute);
        }
        validate_fingerprint(&self.fingerprint)?;
        validate_unique_capabilities(&self.required_capabilities)?;
        validate_unique_note_sockets(&self.inputs, NoteSocketDirection::Input)?;
        validate_unique_note_sockets(&self.outputs, NoteSocketDirection::Output)?;
        if self.outputs.is_empty() {
            return Err(ApproachNoteValidationError::MissingOutput);
        }
        Ok(())
    }
}

/// Exact local Approach Note content pinned by an accepted Blueprint Application.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BlueprintApproachNotePin {
    pub approach_id: String,
    pub absolute_path: String,
    pub fingerprint: String,
}

impl BlueprintApproachNotePin {
    /// Validate the stable identity and immutable local content locator.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the pin cannot safely be replayed.
    pub fn validate(&self) -> Result<(), ApproachNoteValidationError> {
        validate_identifier(&self.approach_id, "approach_id")
            .map_err(ApproachNoteValidationError::InvalidIdentity)?;
        if !Path::new(&self.absolute_path).is_absolute() {
            return Err(ApproachNoteValidationError::PathNotAbsolute);
        }
        validate_fingerprint(&self.fingerprint)
    }
}

/// Durable human intent to apply one portable Blueprint revision to concrete work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlueprintApplication {
    pub application_id: String,
    pub blueprint_id: String,
    pub revision_id: String,
    pub entry_node_id: String,
    pub project_id: String,
    pub work_item_id: String,
}

/// Concrete local facts resolved only after a Blueprint Application is accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlueprintRuntimeBinding {
    pub application_id: String,
    pub project_id: String,
    pub work_item_id: String,
    pub agent_profile_id: String,
    pub execution_workspace: ExecutionWorkspaceConnection,
    pub approach_notes: Vec<BlueprintApproachNotePin>,
}

/// Atomic result of accepting one Blueprint Application and pinning its runtime facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlueprintApplicationReceipt {
    pub application: BlueprintApplication,
    pub runtime_binding: BlueprintRuntimeBinding,
    pub created: bool,
}

/// Structural failure that prevents a Blueprint from being safely applied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlueprintValidationError {
    InvalidIdentifier {
        field: &'static str,
    },
    InvalidName,
    MissingNode,
    MissingTerminal,
    MissingScopeWork {
        scope: BlueprintScope,
    },
    DuplicateNode {
        node_id: String,
    },
    DuplicateLink {
        link_id: String,
    },
    NodeNotFound {
        node_id: String,
    },
    InvalidNodeForScope {
        node_id: String,
        scope: BlueprintScope,
    },
    InvalidTargetReference {
        node_id: String,
    },
    DuplicateSocket {
        node_id: String,
        direction: NoteSocketDirection,
        socket: NoteSocketKind,
    },
    TerminalHasOutput {
        node_id: String,
    },
    AmbiguousFlow {
        source_node_id: String,
        signal: ControlSignal,
    },
    InvalidFlowSignal {
        node_id: String,
        signal: ControlSignal,
    },
    DuplicateDataLink {
        source_node_id: String,
        destination_node_id: String,
        socket: NoteSocketKind,
    },
    IncompatibleSocket {
        source_node_id: String,
        destination_node_id: String,
        socket: NoteSocketKind,
    },
    UnreachableNode {
        node_id: String,
    },
}

impl fmt::Display for BlueprintValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Orchestration Blueprint is invalid: {self:?}")
    }
}

impl std::error::Error for BlueprintValidationError {}

/// Malformed metadata or locator for one Approach Note.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApproachNoteValidationError {
    InvalidIdentity(BlueprintValidationError),
    InvalidTitle,
    PathNotAbsolute,
    InvalidFingerprint,
    InvalidCapability,
    DuplicateCapability {
        capability: String,
    },
    DuplicateSocket {
        direction: NoteSocketDirection,
        socket: NoteSocketKind,
    },
    MissingOutput,
}

impl fmt::Display for ApproachNoteValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Approach Note manifest is invalid: {self:?}")
    }
}

impl std::error::Error for ApproachNoteValidationError {}

/// Side of an Approach Note on which a socket was declared.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoteSocketDirection {
    Input,
    Output,
}

fn validate_node_kind(
    blueprint: &OrchestrationBlueprintRevision,
    node: &BlueprintNode,
) -> Result<(), BlueprintValidationError> {
    let allowed = match blueprint.scope {
        BlueprintScope::Project => !matches!(node.kind, BlueprintNodeKind::ProjectSelector { .. }),
        BlueprintScope::Portfolio => !matches!(node.kind, BlueprintNodeKind::Approach { .. }),
    };
    if !allowed {
        return Err(BlueprintValidationError::InvalidNodeForScope {
            node_id: node.id.clone(),
            scope: blueprint.scope,
        });
    }
    match &node.kind {
        BlueprintNodeKind::Approach { approach_id } => {
            validate_identifier(approach_id, "approach_id")?;
        }
        BlueprintNodeKind::ProjectSelector {
            target_blueprint_id,
            target_revision_id,
            target_entry_id,
        } => {
            validate_identifier(target_blueprint_id, "target_blueprint_id")?;
            validate_identifier(target_revision_id, "target_revision_id")?;
            validate_identifier(target_entry_id, "target_entry_id")?;
            if target_blueprint_id == &blueprint.blueprint_id
                && target_revision_id == &blueprint.revision_id
            {
                return Err(BlueprintValidationError::InvalidTargetReference {
                    node_id: node.id.clone(),
                });
            }
        }
        BlueprintNodeKind::Gate
        | BlueprintNodeKind::Audit
        | BlueprintNodeKind::Approval
        | BlueprintNodeKind::PostAction { .. }
        | BlueprintNodeKind::Terminal => {}
    }
    Ok(())
}

fn validate_sockets(node: &BlueprintNode) -> Result<(), BlueprintValidationError> {
    validate_node_socket_direction(node, &node.inputs, NoteSocketDirection::Input)?;
    validate_node_socket_direction(node, &node.outputs, NoteSocketDirection::Output)?;
    if matches!(node.kind, BlueprintNodeKind::Terminal) && !node.outputs.is_empty() {
        return Err(BlueprintValidationError::TerminalHasOutput {
            node_id: node.id.clone(),
        });
    }
    Ok(())
}

fn validate_node_socket_direction(
    node: &BlueprintNode,
    sockets: &[NoteSocketKind],
    direction: NoteSocketDirection,
) -> Result<(), BlueprintValidationError> {
    let mut seen = HashSet::with_capacity(sockets.len());
    for socket in sockets {
        if !seen.insert(*socket) {
            return Err(BlueprintValidationError::DuplicateSocket {
                node_id: node.id.clone(),
                direction,
                socket: *socket,
            });
        }
    }
    Ok(())
}

fn find_node<'a>(
    nodes: &'a [BlueprintNode],
    node_id: &str,
) -> Result<&'a BlueprintNode, BlueprintValidationError> {
    nodes.iter().find(|node| node.id == node_id).ok_or_else(|| {
        BlueprintValidationError::NodeNotFound {
            node_id: node_id.to_owned(),
        }
    })
}

fn validate_flow_authority(
    source: &BlueprintNode,
    signal: ControlSignal,
) -> Result<(), BlueprintValidationError> {
    let valid = match source.kind {
        BlueprintNodeKind::Approval => {
            matches!(signal, ControlSignal::Approved | ControlSignal::Rejected)
        }
        BlueprintNodeKind::Terminal => false,
        _ => !matches!(signal, ControlSignal::Approved),
    };
    if valid {
        Ok(())
    } else {
        Err(BlueprintValidationError::InvalidFlowSignal {
            node_id: source.id.clone(),
            signal,
        })
    }
}

fn validate_unique_capabilities(
    capabilities: &[String],
) -> Result<(), ApproachNoteValidationError> {
    let mut seen = HashSet::with_capacity(capabilities.len());
    for capability in capabilities {
        validate_identifier(capability, "capability")
            .map_err(|_| ApproachNoteValidationError::InvalidCapability)?;
        if !seen.insert(capability.as_str()) {
            return Err(ApproachNoteValidationError::DuplicateCapability {
                capability: capability.clone(),
            });
        }
    }
    Ok(())
}

fn validate_unique_note_sockets(
    sockets: &[NoteSocketKind],
    direction: NoteSocketDirection,
) -> Result<(), ApproachNoteValidationError> {
    let mut seen = HashSet::with_capacity(sockets.len());
    for socket in sockets {
        if !seen.insert(*socket) {
            return Err(ApproachNoteValidationError::DuplicateSocket {
                direction,
                socket: *socket,
            });
        }
    }
    Ok(())
}

fn validate_fingerprint(value: &str) -> Result<(), ApproachNoteValidationError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(ApproachNoteValidationError::InvalidFingerprint)
    }
}

fn validate_name(value: &str) -> Result<(), BlueprintValidationError> {
    if value.is_empty() || value.trim() != value || value.chars().count() > 128 {
        Err(BlueprintValidationError::InvalidName)
    } else {
        Ok(())
    }
}

fn validate_identifier(value: &str, field: &'static str) -> Result<(), BlueprintValidationError> {
    let length = value.chars().count();
    let valid_first = value
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_lowercase());
    let valid_last = value
        .chars()
        .last()
        .is_some_and(|character| character.is_ascii_lowercase() || character.is_ascii_digit());
    let valid_chars = value.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '-' | '_')
    });
    if length == 0 || length > 64 || !valid_first || !valid_last || !valid_chars {
        Err(BlueprintValidationError::InvalidIdentifier { field })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ApproachNoteManifest, ApproachRisk, BlueprintLink, BlueprintLinkKind, BlueprintNode,
        BlueprintNodeKind, BlueprintScope, BlueprintValidationError, NoteSocketKind,
        OrchestrationBlueprintRevision,
    };
    use crate::ControlSignal;

    #[test]
    fn accepts_project_independent_approach_with_typed_data() {
        let blueprint = project_blueprint();

        assert_eq!(blueprint.validate(), Ok(()));
        assert_eq!(blueprint.approach_ids(), vec!["evidence-first"]);
    }

    #[test]
    fn rejects_concrete_project_scope_and_incompatible_sockets() {
        let mut blueprint = project_blueprint();
        blueprint.scope = BlueprintScope::Portfolio;
        assert!(matches!(
            blueprint.validate(),
            Err(BlueprintValidationError::InvalidNodeForScope { .. })
        ));

        let mut blueprint = project_blueprint();
        blueprint.links.push(BlueprintLink {
            id: "invalid-data".to_owned(),
            source_node_id: "approach".to_owned(),
            destination_node_id: "finish".to_owned(),
            kind: BlueprintLinkKind::Data {
                socket: NoteSocketKind::Evidence,
            },
        });
        assert!(matches!(
            blueprint.validate(),
            Err(BlueprintValidationError::IncompatibleSocket { .. })
        ));
    }

    #[test]
    fn validates_pinned_approach_note_metadata() {
        let manifest = ApproachNoteManifest {
            approach_id: "evidence-first".to_owned(),
            title: "Evidence first".to_owned(),
            absolute_path: std::env::current_dir()
                .unwrap()
                .join("evidence-first.md")
                .display()
                .to_string(),
            fingerprint: "a".repeat(64),
            required_capabilities: vec!["research".to_owned()],
            inputs: vec![NoteSocketKind::Context, NoteSocketKind::WorkItem],
            outputs: vec![NoteSocketKind::Evidence],
            risk: ApproachRisk::Low,
        };

        assert_eq!(manifest.validate(), Ok(()));
    }

    fn project_blueprint() -> OrchestrationBlueprintRevision {
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
                        approach_id: "evidence-first".to_owned(),
                    },
                    inputs: vec![NoteSocketKind::Context, NoteSocketKind::WorkItem],
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
