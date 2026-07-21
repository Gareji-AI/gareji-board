use std::collections::{HashMap, HashSet};

use gareji_board_domain::{
    AgentProfileSummary, ApproachNoteManifest, ApproachNoteValidationError, ApproachRisk,
    AutopilotStopReason, BlueprintNodeKind, BlueprintScope, BlueprintValidationError,
    ExecutionWorkspaceConnection, NoCandidateReason, OrchestrationBlueprintRevision,
    PortfolioSnapshot, ProjectSummary, SafeAutopilotOutcome, SafeAutopilotPreview, WorkItemSummary,
};

/// Current Board facts required to preview portable Blueprint applications.
pub struct BlueprintPlanningFacts<'a> {
    pub blueprints: &'a [OrchestrationBlueprintRevision],
    pub approach_notes: &'a [ApproachNoteManifest],
    pub portfolio: &'a PortfolioSnapshot,
    pub work_items: &'a [WorkItemSummary],
    pub agent_profiles: &'a [AgentProfileSummary],
    pub execution_workspaces: &'a [ExecutionWorkspaceConnection],
}

/// Pure deterministic Module that matches reusable Blueprints to managed projects.
pub struct BlueprintApplicationPlanner;

impl BlueprintApplicationPlanner {
    /// Preview every valid project application without storing intent or starting a Runner.
    #[must_use]
    pub fn preview(facts: &BlueprintPlanningFacts<'_>) -> BlueprintApplicationPreview {
        let note_index = index_notes(facts.approach_notes);
        let mut candidates = Vec::new();
        let mut blocked = Vec::new();

        for blueprint in facts.blueprints {
            let prepared = match PreparedBlueprint::new(blueprint, &note_index) {
                Ok(prepared) => prepared,
                Err(reason) => {
                    blocked.push(BlueprintApplicationBlock {
                        blueprint_id: blueprint.blueprint_id.clone(),
                        revision_id: blueprint.revision_id.clone(),
                        project_id: None,
                        reason,
                    });
                    continue;
                }
            };
            for project in &facts.portfolio.projects {
                match preview_project(&prepared, project, facts) {
                    Ok(candidate) => candidates.push(candidate),
                    Err(reason) => blocked.push(BlueprintApplicationBlock {
                        blueprint_id: blueprint.blueprint_id.clone(),
                        revision_id: blueprint.revision_id.clone(),
                        project_id: Some(project.id.clone()),
                        reason,
                    }),
                }
            }
        }

        candidates.sort_by(compare_candidates);
        blocked.sort_by(|left, right| {
            left.blueprint_id
                .cmp(&right.blueprint_id)
                .then_with(|| left.revision_id.cmp(&right.revision_id))
                .then_with(|| left.project_id.cmp(&right.project_id))
        });
        BlueprintApplicationPreview {
            candidates,
            blocked,
        }
    }
}

/// Complete read-only result of one Blueprint matching pass.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BlueprintApplicationPreview {
    pub candidates: Vec<BlueprintApplicationProposal>,
    pub blocked: Vec<BlueprintApplicationBlock>,
}

impl BlueprintApplicationPreview {
    /// Return the deterministic first proposal without accepting it.
    #[must_use]
    pub fn preferred(&self) -> Option<&BlueprintApplicationProposal> {
        self.candidates.first()
    }
}

/// One inspectable proposal to apply a Blueprint to concrete current work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlueprintApplicationProposal {
    pub blueprint_id: String,
    pub revision_id: String,
    pub entry_node_id: String,
    pub blueprint_name: String,
    pub project_id: String,
    pub project_name: String,
    pub work_item: WorkItemSummary,
    pub agent_profile_id: String,
    pub agent_role: String,
    pub execution_workspace: ExecutionWorkspaceConnection,
    pub approach_notes: Vec<ApproachNotePin>,
    pub required_capabilities: Vec<String>,
    pub highest_risk: ApproachRisk,
    pub active_runs: u32,
    pub execution_cap: u32,
}

/// Exact local knowledge revision that an accepted Runtime Binding would pin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproachNotePin {
    pub approach_id: String,
    pub absolute_path: String,
    pub fingerprint: String,
}

/// One Blueprint/project pair that failed closed during matching.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlueprintApplicationBlock {
    pub blueprint_id: String,
    pub revision_id: String,
    pub project_id: Option<String>,
    pub reason: BlueprintApplicationBlockedReason,
}

/// Bounded reason why a Blueprint cannot currently be proposed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlueprintApplicationBlockedReason {
    InvalidBlueprint(BlueprintValidationError),
    BlueprintScopeNotProject,
    DuplicateApproachNote {
        approach_id: String,
    },
    ApproachNoteNotFound {
        approach_id: String,
    },
    InvalidApproachNote {
        approach_id: String,
        error: ApproachNoteValidationError,
    },
    SocketContractMismatch {
        node_id: String,
        approach_id: String,
    },
    ExecutionWorkspaceNotFound,
    NoCandidate(NoCandidateReason),
    AutopilotStopped(AutopilotStopReason),
    SelectedWorkItemNotFound {
        work_item_id: String,
    },
}

struct PreparedBlueprint<'a> {
    revision: &'a OrchestrationBlueprintRevision,
    notes: Vec<&'a ApproachNoteManifest>,
    required_capabilities: Vec<String>,
    highest_risk: ApproachRisk,
}

impl<'a> PreparedBlueprint<'a> {
    fn new(
        revision: &'a OrchestrationBlueprintRevision,
        note_index: &'a HashMap<&str, NoteIndexEntry<'a>>,
    ) -> Result<Self, BlueprintApplicationBlockedReason> {
        revision
            .validate()
            .map_err(BlueprintApplicationBlockedReason::InvalidBlueprint)?;
        if revision.scope != BlueprintScope::Project {
            return Err(BlueprintApplicationBlockedReason::BlueprintScopeNotProject);
        }
        let mut notes = Vec::new();
        let mut seen_notes = HashSet::new();
        let mut required_capabilities = HashSet::new();
        let mut highest_risk = ApproachRisk::Low;
        for node in &revision.nodes {
            let BlueprintNodeKind::Approach { approach_id } = &node.kind else {
                continue;
            };
            let note = resolve_note(note_index, approach_id)?;
            note.validate().map_err(|error| {
                BlueprintApplicationBlockedReason::InvalidApproachNote {
                    approach_id: approach_id.clone(),
                    error,
                }
            })?;
            if socket_set(&node.inputs) != socket_set(&note.inputs)
                || socket_set(&node.outputs) != socket_set(&note.outputs)
            {
                return Err(BlueprintApplicationBlockedReason::SocketContractMismatch {
                    node_id: node.id.clone(),
                    approach_id: approach_id.clone(),
                });
            }
            if seen_notes.insert(note.approach_id.as_str()) {
                notes.push(note);
            }
            required_capabilities.extend(note.required_capabilities.iter().cloned());
            highest_risk = highest_risk.max(note.risk);
        }
        notes.sort_by(|left, right| left.approach_id.cmp(&right.approach_id));
        let mut required_capabilities = required_capabilities.into_iter().collect::<Vec<_>>();
        required_capabilities.sort();
        Ok(Self {
            revision,
            notes,
            required_capabilities,
            highest_risk,
        })
    }
}

#[derive(Clone, Copy)]
enum NoteIndexEntry<'a> {
    One(&'a ApproachNoteManifest),
    Duplicate,
}

fn index_notes(notes: &[ApproachNoteManifest]) -> HashMap<&str, NoteIndexEntry<'_>> {
    let mut index = HashMap::with_capacity(notes.len());
    for note in notes {
        index
            .entry(note.approach_id.as_str())
            .and_modify(|entry| *entry = NoteIndexEntry::Duplicate)
            .or_insert(NoteIndexEntry::One(note));
    }
    index
}

fn resolve_note<'a>(
    note_index: &'a HashMap<&str, NoteIndexEntry<'a>>,
    approach_id: &str,
) -> Result<&'a ApproachNoteManifest, BlueprintApplicationBlockedReason> {
    match note_index.get(approach_id) {
        Some(NoteIndexEntry::One(note)) => Ok(note),
        Some(NoteIndexEntry::Duplicate) => {
            Err(BlueprintApplicationBlockedReason::DuplicateApproachNote {
                approach_id: approach_id.to_owned(),
            })
        }
        None => Err(BlueprintApplicationBlockedReason::ApproachNoteNotFound {
            approach_id: approach_id.to_owned(),
        }),
    }
}

fn preview_project(
    prepared: &PreparedBlueprint<'_>,
    project: &ProjectSummary,
    facts: &BlueprintPlanningFacts<'_>,
) -> Result<BlueprintApplicationProposal, BlueprintApplicationBlockedReason> {
    let workspace = facts
        .execution_workspaces
        .iter()
        .find(|workspace| workspace.project_id == project.id)
        .ok_or(BlueprintApplicationBlockedReason::ExecutionWorkspaceNotFound)?;
    let original_work_items = facts
        .work_items
        .iter()
        .filter(|work_item| work_item.project_id == project.id)
        .collect::<Vec<_>>();
    let augmented_work_items = facts
        .work_items
        .iter()
        .map(|work_item| {
            if work_item.project_id == project.id {
                augment_work_item(work_item, &prepared.required_capabilities)
            } else {
                work_item.clone()
            }
        })
        .collect::<Vec<_>>();
    let autopilot = SafeAutopilotPreview::evaluate_for_project(
        facts.portfolio,
        &augmented_work_items,
        facts.agent_profiles,
        u32::MAX,
        &project.id,
    );
    let candidate = match autopilot.outcome {
        SafeAutopilotOutcome::Candidate(candidate) => candidate,
        SafeAutopilotOutcome::NoCandidate(reason) => {
            return Err(BlueprintApplicationBlockedReason::NoCandidate(reason));
        }
        SafeAutopilotOutcome::Stop(reason) => {
            return Err(BlueprintApplicationBlockedReason::AutopilotStopped(reason));
        }
    };
    let original_work_item = original_work_items
        .into_iter()
        .find(|work_item| work_item.id == candidate.work_item.id)
        .ok_or_else(
            || BlueprintApplicationBlockedReason::SelectedWorkItemNotFound {
                work_item_id: candidate.work_item.id.clone(),
            },
        )?;
    Ok(BlueprintApplicationProposal {
        blueprint_id: prepared.revision.blueprint_id.clone(),
        revision_id: prepared.revision.revision_id.clone(),
        entry_node_id: prepared.revision.entry_node_id.clone(),
        blueprint_name: prepared.revision.name.clone(),
        project_id: project.id.clone(),
        project_name: project.name.clone(),
        work_item: original_work_item.clone(),
        agent_profile_id: candidate.agent_profile_id,
        agent_role: candidate.agent_role,
        execution_workspace: workspace.clone(),
        approach_notes: prepared
            .notes
            .iter()
            .map(|note| ApproachNotePin {
                approach_id: note.approach_id.clone(),
                absolute_path: note.absolute_path.clone(),
                fingerprint: note.fingerprint.clone(),
            })
            .collect(),
        required_capabilities: prepared.required_capabilities.clone(),
        highest_risk: prepared.highest_risk,
        active_runs: project.work_items.in_progress,
        execution_cap: project.execution_cap,
    })
}

fn augment_work_item(
    work_item: &WorkItemSummary,
    blueprint_capabilities: &[String],
) -> WorkItemSummary {
    let mut work_item = work_item.clone();
    work_item
        .required_capabilities
        .extend(blueprint_capabilities.iter().cloned());
    work_item.required_capabilities.sort();
    work_item.required_capabilities.dedup();
    work_item
}

fn socket_set(
    sockets: &[gareji_board_domain::NoteSocketKind],
) -> HashSet<gareji_board_domain::NoteSocketKind> {
    sockets.iter().copied().collect()
}

fn compare_candidates(
    left: &BlueprintApplicationProposal,
    right: &BlueprintApplicationProposal,
) -> std::cmp::Ordering {
    let left_load = u64::from(left.active_runs) * u64::from(right.execution_cap);
    let right_load = u64::from(right.active_runs) * u64::from(left.execution_cap);
    left_load
        .cmp(&right_load)
        .then_with(|| left.work_item.priority.cmp(&right.work_item.priority))
        .then_with(|| left.project_id.cmp(&right.project_id))
        .then_with(|| left.work_item.id.cmp(&right.work_item.id))
        .then_with(|| left.blueprint_id.cmp(&right.blueprint_id))
        .then_with(|| left.revision_id.cmp(&right.revision_id))
}

#[cfg(test)]
mod tests {
    use gareji_board_domain::{
        AgentProfileSummary, ApproachNoteManifest, ApproachRisk, ApprovalRequirement,
        BlueprintLink, BlueprintLinkKind, BlueprintNode, BlueprintNodeKind, BlueprintScope,
        ControlSignal, ExecutionWorkspaceConnection, ExecutionWorkspaceKind, NoteSocketKind,
        OrchestrationBlueprintRevision, PortfolioSnapshot, ProjectHealth, ProjectSummary,
        WorkItemCounts, WorkItemState, WorkItemSummary,
    };

    use super::{
        BlueprintApplicationBlockedReason, BlueprintApplicationPlanner, BlueprintPlanningFacts,
    };

    #[test]
    fn proposes_one_portable_blueprint_across_managed_projects() {
        let blueprint = blueprint();
        let note = note();
        let portfolio = PortfolioSnapshot {
            projects: vec![project("board", 1), project("core", 1)],
        };
        let work_items = vec![
            work_item("BOARD-1", "board", 2),
            work_item("CORE-1", "core", 1),
        ];
        let profiles = profiles();
        let workspaces = vec![workspace("board"), workspace("core")];

        let preview = BlueprintApplicationPlanner::preview(&BlueprintPlanningFacts {
            blueprints: &[blueprint],
            approach_notes: &[note],
            portfolio: &portfolio,
            work_items: &work_items,
            agent_profiles: &profiles,
            execution_workspaces: &workspaces,
        });

        assert_eq!(preview.candidates.len(), 2);
        let preferred = preview.preferred().unwrap();
        assert_eq!(preferred.project_id, "core");
        assert_eq!(preferred.work_item.id, "CORE-1");
        assert_eq!(preferred.agent_profile_id, "researcher");
        assert_eq!(preferred.approach_notes[0].approach_id, "evidence-first");
        assert!(preview.blocked.is_empty());
    }

    #[test]
    fn fails_closed_for_missing_workspace_and_socket_drift() {
        let mut blueprint = blueprint();
        let note = note();
        let portfolio = PortfolioSnapshot {
            projects: vec![project("board", 1)],
        };
        let work_items = vec![work_item("BOARD-1", "board", 1)];
        let profiles = profiles();

        let preview = BlueprintApplicationPlanner::preview(&BlueprintPlanningFacts {
            blueprints: &[blueprint.clone()],
            approach_notes: std::slice::from_ref(&note),
            portfolio: &portfolio,
            work_items: &work_items,
            agent_profiles: &profiles,
            execution_workspaces: &[],
        });
        assert!(matches!(
            preview.blocked[0].reason,
            BlueprintApplicationBlockedReason::ExecutionWorkspaceNotFound
        ));

        blueprint.nodes[0].outputs = vec![NoteSocketKind::Artifact];
        let preview = BlueprintApplicationPlanner::preview(&BlueprintPlanningFacts {
            blueprints: &[blueprint],
            approach_notes: &[note],
            portfolio: &portfolio,
            work_items: &work_items,
            agent_profiles: &profiles,
            execution_workspaces: &[workspace("board")],
        });
        assert!(matches!(
            preview.blocked[0].reason,
            BlueprintApplicationBlockedReason::SocketContractMismatch { .. }
        ));
    }

    fn blueprint() -> OrchestrationBlueprintRevision {
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

    fn note() -> ApproachNoteManifest {
        ApproachNoteManifest {
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
        }
    }

    fn project(id: &str, cap: u32) -> ProjectSummary {
        ProjectSummary {
            id: id.to_owned(),
            name: id.to_owned(),
            health: ProjectHealth::Healthy,
            execution_cap: cap,
            work_items: WorkItemCounts {
                total: 1,
                todo: 1,
                ..WorkItemCounts::default()
            },
        }
    }

    fn work_item(id: &str, project_id: &str, priority: u32) -> WorkItemSummary {
        WorkItemSummary {
            id: id.to_owned(),
            project_id: project_id.to_owned(),
            title: id.to_owned(),
            priority,
            state: WorkItemState::Todo,
            approval_requirement: ApprovalRequirement::None,
            dependency_ids: Vec::new(),
            agent_profile_id: Some("researcher".to_owned()),
            required_capabilities: Vec::new(),
        }
    }

    fn profiles() -> Vec<AgentProfileSummary> {
        vec![AgentProfileSummary {
            id: "researcher".to_owned(),
            role: "Researcher".to_owned(),
            capabilities: vec!["research".to_owned(), "testing".to_owned()],
            instruction_ref: None,
            skill_refs: Vec::new(),
        }]
    }

    fn workspace(project_id: &str) -> ExecutionWorkspaceConnection {
        ExecutionWorkspaceConnection {
            project_id: project_id.to_owned(),
            kind: ExecutionWorkspaceKind::BundledSample,
            location: None,
        }
    }
}
