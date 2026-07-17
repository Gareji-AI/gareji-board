use std::fs;
use std::path::{Component, Path, PathBuf};

use gareji_board_domain::AgentProfileSummary;

const BUNDLED_INSTRUCTIONS: &[(&str, &str)] = &[
    (
        "agents/implementer/AGENT.md",
        include_str!("../../../examples/demo-execution-workspace/agents/implementer/AGENT.md"),
    ),
    (
        "agents/researcher/AGENT.md",
        include_str!("../../../examples/demo-execution-workspace/agents/researcher/AGENT.md"),
    ),
    (
        "agents/reviewer/AGENT.md",
        include_str!("../../../examples/demo-execution-workspace/agents/reviewer/AGENT.md"),
    ),
    (
        "agents/release-checker/AGENT.md",
        include_str!("../../../examples/demo-execution-workspace/agents/release-checker/AGENT.md"),
    ),
];

const BUNDLED_SKILLS: &[(&str, &str)] = &[
    (
        "implement-bounded-work-item",
        include_str!(
            "../../../examples/demo-execution-workspace/.agents/skills/implement-bounded-work-item/SKILL.md"
        ),
    ),
    (
        "review-work-item",
        include_str!(
            "../../../examples/demo-execution-workspace/.agents/skills/review-work-item/SKILL.md"
        ),
    ),
    (
        "summarize-project-context",
        include_str!(
            "../../../examples/demo-execution-workspace/.agents/skills/summarize-project-context/SKILL.md"
        ),
    ),
    (
        "write-project-handoff",
        include_str!(
            "../../../examples/demo-execution-workspace/.agents/skills/write-project-handoff/SKILL.md"
        ),
    ),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentBehaviorInspection {
    pub status: BehaviorInspectionStatus,
    pub instruction: Option<ReferenceInspection>,
    pub skills: Vec<ReferenceInspection>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BehaviorInspectionStatus {
    Ready,
    NeedsAttention,
    Unsafe,
    WorkspaceUnavailable,
    NoReferences,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferenceInspection {
    pub reference: String,
    pub status: ReferenceStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceStatus {
    Present,
    Missing,
    Unsafe,
    NotInspected,
}

#[derive(Clone, Debug)]
pub struct AgentBehaviorInspector {
    source: InspectionSource,
    label: String,
}

#[derive(Clone, Debug)]
enum InspectionSource {
    BundledSample,
    ExecutionWorkspace(Option<PathBuf>),
}

impl AgentBehaviorInspector {
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn inspect(&self, profile: &AgentProfileSummary) -> AgentBehaviorInspection {
        if profile.instruction_ref.is_none() && profile.skill_refs.is_empty() {
            return AgentBehaviorInspection {
                status: BehaviorInspectionStatus::NoReferences,
                instruction: None,
                skills: Vec::new(),
            };
        }

        match &self.source {
            InspectionSource::BundledSample => inspect_bundled(profile),
            InspectionSource::ExecutionWorkspace(root) => {
                inspect_workspace(root.as_deref(), profile)
            }
        }
    }

    pub fn bundled_sample() -> Self {
        Self {
            source: InspectionSource::BundledSample,
            label: "Bundled sample Execution workspace".to_owned(),
        }
    }

    pub fn for_workspace(root: &Path) -> Self {
        let name = root
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("selected folder");
        let canonical_root = fs::canonicalize(root)
            .ok()
            .filter(|candidate| candidate.is_dir());
        Self {
            label: format!("Selected Execution workspace · {name}"),
            source: InspectionSource::ExecutionWorkspace(canonical_root),
        }
    }

    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            source: InspectionSource::ExecutionWorkspace(None),
            label: "No Execution workspace connected".to_owned(),
        }
    }
}

fn inspect_bundled(profile: &AgentProfileSummary) -> AgentBehaviorInspection {
    let instruction = profile
        .instruction_ref
        .as_ref()
        .map(|reference| ReferenceInspection {
            reference: reference.clone(),
            status: if !relative_path_is_safe(Path::new(reference)) {
                ReferenceStatus::Unsafe
            } else if BUNDLED_INSTRUCTIONS
                .iter()
                .any(|(candidate, _)| candidate == reference)
            {
                ReferenceStatus::Present
            } else {
                ReferenceStatus::Missing
            },
        });
    let skills = profile
        .skill_refs
        .iter()
        .map(|reference| ReferenceInspection {
            reference: reference.clone(),
            status: if !stable_identifier_is_safe(reference) {
                ReferenceStatus::Unsafe
            } else if BUNDLED_SKILLS
                .iter()
                .any(|(candidate, _)| candidate == reference)
            {
                ReferenceStatus::Present
            } else {
                ReferenceStatus::Missing
            },
        })
        .collect::<Vec<_>>();
    inspection_from_references(instruction, skills)
}

fn inspect_workspace(
    canonical_root: Option<&Path>,
    profile: &AgentProfileSummary,
) -> AgentBehaviorInspection {
    let Some(canonical_root) = canonical_root else {
        return unavailable_inspection(profile);
    };

    let instruction = profile
        .instruction_ref
        .as_ref()
        .map(|reference| ReferenceInspection {
            reference: reference.clone(),
            status: inspect_file(canonical_root, Path::new(reference)),
        });
    let skills = profile
        .skill_refs
        .iter()
        .map(|reference| {
            let path = PathBuf::from(format!(".agents/skills/{reference}/SKILL.md"));
            ReferenceInspection {
                reference: reference.clone(),
                status: if stable_identifier_is_safe(reference) {
                    inspect_file(canonical_root, &path)
                } else {
                    ReferenceStatus::Unsafe
                },
            }
        })
        .collect::<Vec<_>>();
    inspection_from_references(instruction, skills)
}

fn inspect_file(canonical_root: &Path, relative: &Path) -> ReferenceStatus {
    if !relative_path_is_safe(relative) {
        return ReferenceStatus::Unsafe;
    }
    let candidate = canonical_root.join(relative);
    let canonical_candidate = match fs::canonicalize(candidate) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ReferenceStatus::Missing;
        }
        Err(_) => return ReferenceStatus::NotInspected,
    };
    if !canonical_candidate.starts_with(canonical_root) {
        return ReferenceStatus::Unsafe;
    }
    if canonical_candidate.is_file() {
        ReferenceStatus::Present
    } else {
        ReferenceStatus::Missing
    }
}

fn relative_path_is_safe(path: &Path) -> bool {
    let text = path.to_string_lossy();
    !text.is_empty()
        && !text.contains(['\\', ':'])
        && !text.chars().any(char::is_control)
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn stable_identifier_is_safe(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn inspection_from_references(
    instruction: Option<ReferenceInspection>,
    skills: Vec<ReferenceInspection>,
) -> AgentBehaviorInspection {
    let statuses = instruction
        .iter()
        .chain(&skills)
        .map(|reference| reference.status)
        .collect::<Vec<_>>();
    let status = if statuses.contains(&ReferenceStatus::Unsafe) {
        BehaviorInspectionStatus::Unsafe
    } else if statuses.contains(&ReferenceStatus::NotInspected) {
        BehaviorInspectionStatus::WorkspaceUnavailable
    } else if statuses.contains(&ReferenceStatus::Missing) {
        BehaviorInspectionStatus::NeedsAttention
    } else {
        BehaviorInspectionStatus::Ready
    };
    AgentBehaviorInspection {
        status,
        instruction,
        skills,
    }
}

fn unavailable_inspection(profile: &AgentProfileSummary) -> AgentBehaviorInspection {
    AgentBehaviorInspection {
        status: BehaviorInspectionStatus::WorkspaceUnavailable,
        instruction: profile
            .instruction_ref
            .as_ref()
            .map(|reference| ReferenceInspection {
                reference: reference.clone(),
                status: ReferenceStatus::NotInspected,
            }),
        skills: profile
            .skill_refs
            .iter()
            .map(|reference| ReferenceInspection {
                reference: reference.clone(),
                status: ReferenceStatus::NotInspected,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn bundled_sample_resolves_known_references_without_reading_profile_contents() {
        let inspection = AgentBehaviorInspector::bundled_sample().inspect(&sample_profile());

        assert_eq!(inspection.status, BehaviorInspectionStatus::Ready);
        assert_eq!(
            inspection.instruction.unwrap().status,
            ReferenceStatus::Present
        );
        assert!(
            inspection
                .skills
                .iter()
                .all(|skill| skill.status == ReferenceStatus::Present)
        );
    }

    #[test]
    fn workspace_inspection_distinguishes_present_missing_and_unavailable() {
        let fixture = TempFixture::new();
        fixture.write("agents/implementer/AGENT.md");
        fixture.write(".agents/skills/implement-bounded-work-item/SKILL.md");
        let mut profile = sample_profile();
        profile.skill_refs.push("missing-skill".to_owned());

        let inspection = AgentBehaviorInspector::for_workspace(&fixture.root).inspect(&profile);
        assert_eq!(inspection.status, BehaviorInspectionStatus::NeedsAttention);
        assert_eq!(inspection.skills[0].status, ReferenceStatus::Present);
        assert_eq!(inspection.skills[2].status, ReferenceStatus::Missing);

        let unavailable =
            AgentBehaviorInspector::for_workspace(&fixture.root.join("moved")).inspect(&profile);
        assert_eq!(
            unavailable.status,
            BehaviorInspectionStatus::WorkspaceUnavailable
        );
        assert!(
            unavailable
                .skills
                .iter()
                .all(|skill| skill.status == ReferenceStatus::NotInspected)
        );
    }

    #[test]
    fn unsafe_and_empty_profiles_have_bounded_results() {
        let fixture = TempFixture::new();
        let unsafe_profile = AgentProfileSummary {
            instruction_ref: Some("../AGENT.md".to_owned()),
            skill_refs: Vec::new(),
            ..sample_profile()
        };
        let unsafe_result =
            AgentBehaviorInspector::for_workspace(&fixture.root).inspect(&unsafe_profile);
        assert_eq!(unsafe_result.status, BehaviorInspectionStatus::Unsafe);

        let empty_profile = AgentProfileSummary {
            instruction_ref: None,
            skill_refs: Vec::new(),
            ..sample_profile()
        };
        let empty_result =
            AgentBehaviorInspector::for_workspace(&fixture.root).inspect(&empty_profile);
        assert_eq!(empty_result.status, BehaviorInspectionStatus::NoReferences);
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_cannot_escape_the_workspace() {
        use std::os::unix::fs::symlink;

        let fixture = TempFixture::new();
        let outside = fixture.root.parent().unwrap().join("outside-agent.md");
        fs::write(&outside, "outside").unwrap();
        let link = fixture.root.join("agents/implementer/AGENT.md");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(&outside, &link).unwrap();

        let inspection =
            AgentBehaviorInspector::for_workspace(&fixture.root).inspect(&AgentProfileSummary {
                skill_refs: Vec::new(),
                ..sample_profile()
            });
        assert_eq!(inspection.status, BehaviorInspectionStatus::Unsafe);
        let _ = fs::remove_file(outside);
    }

    fn sample_profile() -> AgentProfileSummary {
        AgentProfileSummary {
            id: "implementer".to_owned(),
            role: "Implementer".to_owned(),
            capabilities: vec!["implementation".to_owned()],
            instruction_ref: Some("agents/implementer/AGENT.md".to_owned()),
            skill_refs: vec![
                "implement-bounded-work-item".to_owned(),
                "write-project-handoff".to_owned(),
            ],
        }
    }

    struct TempFixture {
        root: PathBuf,
    }

    impl TempFixture {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "gareji-board-agent-inspection-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn write(&self, relative: &str) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "fixture").unwrap();
        }
    }

    impl Drop for TempFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
