//! Codex Runner Adapter with isolated Git worktree execution.

use std::collections::BTreeSet;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use directories::ProjectDirs;
use gareji_board_domain::{
    AgentProfileSummary, ApprovalRequirement, ExecutionWorkspaceConnection, ExecutionWorkspaceKind,
    WorkItemState, WorkItemSummary,
};
use serde::{Deserialize, Serialize};

const MAX_ID_CHARS: usize = 128;
const MAX_TITLE_CHARS: usize = 512;
const MAX_ROLE_CHARS: usize = 128;
const MAX_MODEL_CHARS: usize = 128;
const MAX_PROFILE_CHARS: usize = 128;
const MAX_REFERENCES: usize = 32;
const MAX_CHANGED_PATHS: usize = 200;
const MAX_HANDOFF_BYTES: u64 = 65_536;
const MAX_GIT_OUTPUT_BYTES: usize = 65_536;
const MAX_TIMEOUT_SECONDS: u32 = 86_400;
const POLL_INTERVAL: Duration = Duration::from_millis(100);

const HANDOFF_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "required": ["outcome", "summary", "verification", "risks", "next_action"],
  "properties": {
    "outcome": {"enum": ["progress", "completed", "blocked", "failed"]},
    "summary": {"type": "string", "minLength": 1, "maxLength": 2000},
    "verification": {"type": "array", "maxItems": 32, "items": {"type": "string", "maxLength": 512}},
    "risks": {"type": "array", "maxItems": 32, "items": {"type": "string", "maxLength": 512}},
    "next_action": {"type": "string", "minLength": 1, "maxLength": 2048}
  }
}"#;

/// Deep Module that validates one Board request and executes one isolated Codex Run.
pub struct CodexRunner {
    run_root: PathBuf,
    git: Box<dyn GitPort>,
    codex: Box<dyn CodexPort>,
}

impl CodexRunner {
    /// Create a production Runner with an explicit evidence root and Codex binary.
    #[must_use]
    pub fn new(run_root: PathBuf, codex_binary: PathBuf) -> Self {
        Self {
            run_root,
            git: Box::new(SystemGit),
            codex: Box::new(SystemCodex { codex_binary }),
        }
    }

    /// Resolve the local Run root and Codex binary from the current environment.
    ///
    /// `GAREJI_RUN_ROOT` and `GAREJI_CODEX_BIN` are optional explicit overrides.
    ///
    /// # Errors
    ///
    /// Returns a bounded configuration error when no local application-data directory exists.
    pub fn from_environment() -> Result<Self, RunnerConfigurationError> {
        let run_root = match env::var_os("GAREJI_RUN_ROOT") {
            Some(path) => PathBuf::from(path),
            None => ProjectDirs::from("ai", "Gareji", "Gareji Board")
                .map(|directories| directories.data_local_dir().join("runs"))
                .ok_or(RunnerConfigurationError::ApplicationDataUnavailable)?,
        };
        let codex_binary =
            env::var_os("GAREJI_CODEX_BIN").map_or_else(|| PathBuf::from("codex"), PathBuf::from);
        Ok(Self::new(run_root, codex_binary))
    }

    /// Execute one Work item in a new Git worktree and preserve its recovery evidence.
    #[must_use]
    pub fn execute(&mut self, request: &CodexRunRequest) -> CodexRunResult {
        let resolved_model = request.model.resolve();
        let mut result = CodexRunResult::rejected(&request.run_id, resolved_model.clone());

        if let Err(failure) = validate_request(request) {
            result.failure = Some(failure);
            return result;
        }

        let Some(workspace_location) = request.execution_workspace.location.as_deref() else {
            result.failure = Some(RunnerFailure::configuration(
                "workspace_not_connected",
                "The Work item does not have a matching local Execution workspace.",
            ));
            return result;
        };
        let workspace = match self.git.inspect_workspace(Path::new(workspace_location)) {
            Ok(workspace) => workspace,
            Err(failure) => {
                result.failure = Some(failure);
                return result;
            }
        };
        if let Err(failure) = inspect_agent_references(&workspace.canonical_workspace, request) {
            result.failure = Some(failure);
            return result;
        }

        let prepared = match self.prepare_run(request, &workspace, &mut result) {
            Ok(prepared) => prepared,
            Err(failure) => {
                if result.worktree.is_some() {
                    result.disposition = CodexRunDisposition::Failed;
                    self.refresh_git_evidence(&mut result);
                }
                result.failure = Some(failure);
                return result;
            }
        };
        self.invoke_codex(request, &resolved_model, prepared, result)
    }

    fn prepare_run(
        &mut self,
        request: &CodexRunRequest,
        workspace: &GitWorkspace,
        result: &mut CodexRunResult,
    ) -> Result<PreparedRun, RunnerFailure> {
        let run_directory = self.run_root.join(run_directory_name(&request.run_id));
        if run_directory.exists() {
            return Err(RunnerFailure::configuration(
                "run_already_exists",
                "A local Run directory already exists for this Run identity.",
            ));
        }
        let evidence_directory = run_directory.join("evidence");
        if create_private_directory(&evidence_directory).is_err() {
            return Err(RunnerFailure::evidence(
                "evidence_directory_unavailable",
                "The local Run evidence directory could not be created.",
            ));
        }

        let worktree_root = run_directory.join("worktree");
        let branch = format!("gareji/run-{}", branch_segment(&request.run_id));
        self.git.create_worktree(
            &workspace.repository_root,
            &worktree_root,
            &branch,
            &workspace.base_revision,
        )?;

        let run_workspace = worktree_root.join(&workspace.relative_workspace);
        result.worktree = Some(WorktreeEvidence {
            root: display_path(&worktree_root),
            working_directory: display_path(&run_workspace),
            branch,
            base_revision: workspace.base_revision.clone(),
            final_revision: workspace.base_revision.clone(),
            has_uncommitted_changes: false,
            changed_paths: Vec::new(),
            changed_paths_truncated: false,
            preserved: true,
        });

        inspect_agent_references(&run_workspace, request)?;

        let schema_path = evidence_directory.join("handoff-schema.json");
        let events_path = evidence_directory.join("codex-events.jsonl");
        let handoff_path = evidence_directory.join("handoff.json");
        if write_private_file(&schema_path, HANDOFF_SCHEMA.as_bytes()).is_err() {
            return Err(RunnerFailure::evidence(
                "handoff_schema_unavailable",
                "The bounded handoff schema could not be stored.",
            ));
        }

        result.events_path = Some(display_path(&events_path));
        result.handoff_path = Some(display_path(&handoff_path));
        Ok(PreparedRun {
            run_workspace,
            schema_path,
            events_path,
            handoff_path,
        })
    }

    fn invoke_codex(
        &mut self,
        request: &CodexRunRequest,
        resolved_model: &ResolvedModel,
        prepared: PreparedRun,
        mut result: CodexRunResult,
    ) -> CodexRunResult {
        let invocation = CodexInvocation {
            working_directory: prepared.run_workspace,
            profile: request.codex_profile.clone(),
            model: resolved_model.model.clone(),
            prompt: build_prompt(request),
            schema_path: prepared.schema_path,
            events_path: prepared.events_path,
            handoff_path: prepared.handoff_path.clone(),
            timeout: Duration::from_secs(u64::from(request.timeout_seconds)),
        };

        match self.codex.execute(&invocation) {
            Ok(CodexExit::Completed { success: true }) => {
                match read_handoff(&prepared.handoff_path) {
                    Ok(handoff) => {
                        result.disposition = CodexRunDisposition::Succeeded;
                        result.handoff = Some(handoff);
                    }
                    Err(failure) => {
                        result.disposition = CodexRunDisposition::Failed;
                        result.failure = Some(failure);
                    }
                }
            }
            Ok(CodexExit::Completed { success: false }) => {
                result.disposition = CodexRunDisposition::Failed;
                result.failure = Some(RunnerFailure::execution(
                    "codex_exit_nonzero",
                    "Codex exited without completing the Run successfully.",
                ));
            }
            Ok(CodexExit::TimedOut) => {
                result.disposition = CodexRunDisposition::TimedOut;
                result.failure = Some(RunnerFailure::timeout(
                    "codex_timeout",
                    "Codex exceeded the configured Run deadline.",
                ));
            }
            Err(()) => {
                result.disposition = CodexRunDisposition::Failed;
                result.failure = Some(RunnerFailure::execution(
                    "codex_unavailable",
                    "Codex could not be started on this system.",
                ));
            }
        }
        self.refresh_git_evidence(&mut result);
        result
    }

    fn refresh_git_evidence(&mut self, result: &mut CodexRunResult) {
        let Some(worktree) = result.worktree.as_mut() else {
            return;
        };
        let root = Path::new(&worktree.root);
        if let Ok(revision) = self.git.revision(root) {
            worktree.final_revision = revision;
        }
        if let Ok(changes) = self.git.changes(root) {
            worktree.has_uncommitted_changes = !changes.paths.is_empty();
            worktree.changed_paths =
                scope_changed_paths(changes.paths, root, Path::new(&worktree.working_directory));
            worktree.changed_paths_truncated = changes.truncated;
        }
    }

    #[cfg(test)]
    fn with_ports(run_root: PathBuf, git: Box<dyn GitPort>, codex: Box<dyn CodexPort>) -> Self {
        Self {
            run_root,
            git,
            codex,
        }
    }
}

/// Complete Board-owned request for one Codex invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexRunRequest {
    pub run_id: String,
    pub execution_workspace: ExecutionWorkspaceConnection,
    pub work_item: WorkItemSummary,
    pub agent_profile: AgentProfileSummary,
    pub model: CodexModelSelection,
    pub codex_profile: Option<String>,
    pub timeout_seconds: u32,
}

/// Ordered model candidates kept independent from Agent behavior.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CodexModelSelection {
    pub run_override: Option<String>,
    pub agent_override: Option<String>,
    pub workspace_default: Option<String>,
}

impl CodexModelSelection {
    #[must_use]
    pub fn resolve(&self) -> ResolvedModel {
        if let Some(model) = &self.run_override {
            return ResolvedModel {
                model: Some(model.clone()),
                source: ModelSource::RunOverride,
            };
        }
        if let Some(model) = &self.agent_override {
            return ResolvedModel {
                model: Some(model.clone()),
                source: ModelSource::AgentProfile,
            };
        }
        if let Some(model) = &self.workspace_default {
            return ResolvedModel {
                model: Some(model.clone()),
                source: ModelSource::WorkspaceDefault,
            };
        }
        ResolvedModel {
            model: None,
            source: ModelSource::CodexDefault,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResolvedModel {
    pub model: Option<String>,
    pub source: ModelSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    RunOverride,
    AgentProfile,
    WorkspaceDefault,
    CodexDefault,
}

/// Bounded result returned for every attempted or rejected Codex Run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CodexRunResult {
    pub run_id: String,
    pub disposition: CodexRunDisposition,
    pub resolved_model: ResolvedModel,
    pub worktree: Option<WorktreeEvidence>,
    pub events_path: Option<String>,
    pub handoff_path: Option<String>,
    pub handoff: Option<CodexHandoff>,
    pub failure: Option<RunnerFailure>,
}

impl CodexRunResult {
    fn rejected(run_id: &str, resolved_model: ResolvedModel) -> Self {
        Self {
            run_id: run_id.to_owned(),
            disposition: CodexRunDisposition::Rejected,
            resolved_model,
            worktree: None,
            events_path: None,
            handoff_path: None,
            handoff: None,
            failure: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexRunDisposition {
    Rejected,
    Succeeded,
    Failed,
    TimedOut,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorktreeEvidence {
    pub root: String,
    pub working_directory: String,
    pub branch: String,
    pub base_revision: String,
    pub final_revision: String,
    pub has_uncommitted_changes: bool,
    pub changed_paths: Vec<String>,
    pub changed_paths_truncated: bool,
    /// Always true in v0; cleanup is a later explicit operation.
    pub preserved: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexHandoff {
    pub outcome: HandoffOutcome,
    pub summary: String,
    pub verification: Vec<String>,
    pub risks: Vec<String>,
    pub next_action: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffOutcome {
    Progress,
    Completed,
    Blocked,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RunnerFailure {
    pub category: RunnerFailureCategory,
    pub code: String,
    pub message: String,
}

impl RunnerFailure {
    fn configuration(code: &str, message: &str) -> Self {
        Self::new(RunnerFailureCategory::Configuration, code, message)
    }

    fn execution(code: &str, message: &str) -> Self {
        Self::new(RunnerFailureCategory::Execution, code, message)
    }

    fn timeout(code: &str, message: &str) -> Self {
        Self::new(RunnerFailureCategory::Timeout, code, message)
    }

    fn protocol(code: &str, message: &str) -> Self {
        Self::new(RunnerFailureCategory::Protocol, code, message)
    }

    fn evidence(code: &str, message: &str) -> Self {
        Self::new(RunnerFailureCategory::Evidence, code, message)
    }

    fn new(category: RunnerFailureCategory, code: &str, message: &str) -> Self {
        Self {
            category,
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerFailureCategory {
    Configuration,
    Execution,
    Timeout,
    Protocol,
    Evidence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RunnerConfigurationError {
    #[error("local application-data directory is unavailable")]
    ApplicationDataUnavailable,
}

/// Validate one complete Board-owned request without creating a worktree or Run.
///
/// This is the shared preflight seam for Board controllers and Runner execution.
///
/// # Errors
///
/// Returns a bounded configuration failure when identities, relationships, or
/// request limits do not satisfy the Runner contract.
pub fn validate_request(request: &CodexRunRequest) -> Result<(), RunnerFailure> {
    validate_request_relationships(request)?;
    validate_request_bounds(request)
}

fn validate_request_relationships(request: &CodexRunRequest) -> Result<(), RunnerFailure> {
    if !stable_run_id(&request.run_id)
        || !valid_text_id(&request.work_item.id)
        || !valid_text_id(&request.work_item.project_id)
        || !valid_text_id(&request.agent_profile.id)
    {
        return Err(RunnerFailure::configuration(
            "invalid_identity",
            "The Run, Work item, project, or Agent identity is invalid.",
        ));
    }
    if request.execution_workspace.project_id != request.work_item.project_id
        || request.execution_workspace.kind != ExecutionWorkspaceKind::LocalDirectory
        || request.execution_workspace.location.is_none()
    {
        return Err(RunnerFailure::configuration(
            "workspace_not_connected",
            "The Work item does not have a matching local Execution workspace.",
        ));
    }
    if !matches!(
        request.work_item.state,
        WorkItemState::Todo | WorkItemState::InProgress | WorkItemState::InReview
    ) {
        return Err(RunnerFailure::configuration(
            "work_item_not_eligible",
            "The Work item is not eligible for active Runner work.",
        ));
    }
    if request.work_item.approval_requirement != ApprovalRequirement::None {
        return Err(RunnerFailure::configuration(
            "approval_required",
            "The Work item requires explicit approval before Runner execution.",
        ));
    }
    if request.work_item.agent_profile_id.as_deref() != Some(&request.agent_profile.id) {
        return Err(RunnerFailure::configuration(
            "agent_assignment_mismatch",
            "The selected Agent profile does not match the Work item assignment.",
        ));
    }
    let declared = request
        .agent_profile
        .capabilities
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if request
        .work_item
        .required_capabilities
        .iter()
        .any(|required| !declared.contains(required.as_str()))
    {
        return Err(RunnerFailure::configuration(
            "agent_capability_missing",
            "The assigned Agent profile lacks a required Agent capability.",
        ));
    }
    Ok(())
}

fn validate_request_bounds(request: &CodexRunRequest) -> Result<(), RunnerFailure> {
    if request.timeout_seconds == 0 || request.timeout_seconds > MAX_TIMEOUT_SECONDS {
        return Err(RunnerFailure::configuration(
            "invalid_timeout",
            "The Run timeout must be between 1 and 86400 seconds.",
        ));
    }
    if request.work_item.title.trim().is_empty()
        || request.work_item.title.chars().count() > MAX_TITLE_CHARS
        || !safe_text(&request.work_item.title)
        || request.agent_profile.role.trim().is_empty()
        || request.agent_profile.role.chars().count() > MAX_ROLE_CHARS
        || !safe_text(&request.agent_profile.role)
    {
        return Err(RunnerFailure::configuration(
            "invalid_display_text",
            "The Work item title or Agent role is invalid.",
        ));
    }
    if request.agent_profile.skill_refs.len() > MAX_REFERENCES
        || request.work_item.required_capabilities.len() > MAX_REFERENCES
        || request.agent_profile.capabilities.len() > MAX_REFERENCES
    {
        return Err(RunnerFailure::configuration(
            "too_many_references",
            "The execution request contains too many bounded references.",
        ));
    }
    for model in [
        request.model.run_override.as_deref(),
        request.model.agent_override.as_deref(),
        request.model.workspace_default.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if !valid_runtime_token(model, MAX_MODEL_CHARS) {
            return Err(RunnerFailure::configuration(
                "invalid_model",
                "A configured Codex model identifier is invalid.",
            ));
        }
    }
    if request
        .codex_profile
        .as_deref()
        .is_some_and(|profile| !valid_runtime_token(profile, MAX_PROFILE_CHARS))
    {
        return Err(RunnerFailure::configuration(
            "invalid_codex_profile",
            "The configured Codex profile identifier is invalid.",
        ));
    }
    Ok(())
}

fn inspect_agent_references(root: &Path, request: &CodexRunRequest) -> Result<(), RunnerFailure> {
    if let Some(reference) = &request.agent_profile.instruction_ref
        && (!safe_relative_path(Path::new(reference))
            || !safe_existing_file(root, Path::new(reference)))
    {
        return Err(RunnerFailure::configuration(
            "agent_instruction_unavailable",
            "The Agent instruction reference is missing or unsafe in the Run workspace.",
        ));
    }
    for skill in &request.agent_profile.skill_refs {
        if !stable_identifier(skill) {
            return Err(RunnerFailure::configuration(
                "skill_reference_unsafe",
                "An Agent Skill reference is invalid.",
            ));
        }
        let relative = PathBuf::from(format!(".agents/skills/{skill}/SKILL.md"));
        if !safe_existing_file(root, &relative) {
            return Err(RunnerFailure::configuration(
                "skill_unavailable",
                "An Agent Skill reference is missing or unsafe in the Run workspace.",
            ));
        }
    }
    Ok(())
}

fn safe_existing_file(root: &Path, relative: &Path) -> bool {
    let Ok(canonical_root) = fs::canonicalize(root) else {
        return false;
    };
    let Ok(candidate) = fs::canonicalize(canonical_root.join(relative)) else {
        return false;
    };
    candidate.starts_with(&canonical_root) && candidate.is_file()
}

fn safe_relative_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    !text.is_empty()
        && !text.contains(['\\', ':'])
        && !text.chars().any(char::is_control)
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn stable_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn valid_text_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_ID_CHARS
        && !value.chars().any(char::is_control)
}

fn stable_run_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn valid_runtime_token(value: &str, max_chars: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max_chars
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        })
}

fn safe_text(value: &str) -> bool {
    value
        .chars()
        .all(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
}

fn build_prompt(request: &CodexRunRequest) -> String {
    let instruction = request
        .agent_profile
        .instruction_ref
        .as_deref()
        .unwrap_or("none");
    let skills = if request.agent_profile.skill_refs.is_empty() {
        "none".to_owned()
    } else {
        request.agent_profile.skill_refs.join(", ")
    };
    format!(
        "Execute one Gareji Board Work item inside this isolated Git worktree.\n\
         Work item ID: {}\n\
         Work item title: {}\n\
         Agent role: {}\n\
         Agent instruction file: {instruction}\n\
         Enabled Skill references: {skills}\n\
         Read the referenced instruction and Skill files before changing code. Stay inside this worktree. Do not push, publish, retrieve secrets, or alter the connected checkout. Verify the bounded change and return only the JSON handoff required by the supplied schema.",
        request.work_item.id, request.work_item.title, request.agent_profile.role
    )
}

fn run_directory_name(run_id: &str) -> String {
    format!("run-{}", branch_segment(run_id))
}

fn branch_segment(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') {
                char::from(byte.to_ascii_lowercase())
            } else {
                '-'
            }
        })
        .collect()
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn write_private_file(path: &Path, content: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(content)
}

fn read_handoff(path: &Path) -> Result<CodexHandoff, RunnerFailure> {
    let metadata = fs::metadata(path).map_err(|_| {
        RunnerFailure::protocol(
            "handoff_missing",
            "Codex did not produce the required bounded handoff.",
        )
    })?;
    if metadata.len() > MAX_HANDOFF_BYTES {
        return Err(RunnerFailure::protocol(
            "handoff_too_large",
            "The Codex handoff exceeded its size limit.",
        ));
    }
    let bytes = fs::read(path).map_err(|_| {
        RunnerFailure::protocol("handoff_unreadable", "The Codex handoff could not be read.")
    })?;
    let handoff: CodexHandoff = serde_json::from_slice(&bytes).map_err(|_| {
        RunnerFailure::protocol(
            "handoff_invalid",
            "Codex returned a handoff that did not match the required schema.",
        )
    })?;
    if handoff.summary.trim().is_empty()
        || handoff.summary.chars().count() > 2_000
        || handoff.next_action.trim().is_empty()
        || handoff.next_action.chars().count() > 2_048
        || handoff.verification.len() > MAX_REFERENCES
        || handoff.risks.len() > MAX_REFERENCES
        || !safe_text(&handoff.summary)
        || !safe_text(&handoff.next_action)
        || handoff
            .verification
            .iter()
            .chain(&handoff.risks)
            .any(|entry| entry.chars().count() > 512 || !safe_text(entry))
    {
        return Err(RunnerFailure::protocol(
            "handoff_invalid",
            "Codex returned a handoff outside the bounded contract.",
        ));
    }
    Ok(handoff)
}

struct GitWorkspace {
    canonical_workspace: PathBuf,
    repository_root: PathBuf,
    relative_workspace: PathBuf,
    base_revision: String,
}

trait GitPort {
    fn inspect_workspace(&mut self, workspace: &Path) -> Result<GitWorkspace, RunnerFailure>;
    fn create_worktree(
        &mut self,
        repository: &Path,
        destination: &Path,
        branch: &str,
        base_revision: &str,
    ) -> Result<(), RunnerFailure>;
    fn revision(&mut self, worktree: &Path) -> Result<String, RunnerFailure>;
    fn changes(&mut self, worktree: &Path) -> Result<GitChanges, RunnerFailure>;
}

struct SystemGit;

impl GitPort for SystemGit {
    fn inspect_workspace(&mut self, workspace: &Path) -> Result<GitWorkspace, RunnerFailure> {
        let canonical_workspace = fs::canonicalize(workspace).map_err(|_| {
            RunnerFailure::configuration(
                "workspace_unavailable",
                "The connected Execution workspace is unavailable.",
            )
        })?;
        if !canonical_workspace.is_dir() {
            return Err(RunnerFailure::configuration(
                "workspace_unavailable",
                "The connected Execution workspace is not a directory.",
            ));
        }
        let repository_text = git_output(&canonical_workspace, &["rev-parse", "--show-toplevel"])?;
        let repository_root = fs::canonicalize(repository_text.trim()).map_err(|_| {
            RunnerFailure::configuration(
                "git_repository_unavailable",
                "The Execution workspace Git repository could not be resolved.",
            )
        })?;
        let relative_workspace = canonical_workspace
            .strip_prefix(&repository_root)
            .map(Path::to_path_buf)
            .map_err(|_| {
                RunnerFailure::configuration(
                    "git_repository_mismatch",
                    "The Execution workspace is outside its resolved Git repository.",
                )
            })?;
        let base_revision = git_output(&canonical_workspace, &["rev-parse", "HEAD"])?;
        Ok(GitWorkspace {
            canonical_workspace,
            repository_root,
            relative_workspace,
            base_revision: base_revision.trim().to_owned(),
        })
    }

    fn create_worktree(
        &mut self,
        repository: &Path,
        destination: &Path,
        branch: &str,
        base_revision: &str,
    ) -> Result<(), RunnerFailure> {
        let status = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(["worktree", "add", "-b"])
            .arg(branch)
            .arg(destination)
            .arg(base_revision)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| git_unavailable())?;
        if status.success() {
            Ok(())
        } else {
            Err(RunnerFailure::configuration(
                "worktree_create_failed",
                "Git could not create the isolated Run worktree.",
            ))
        }
    }

    fn revision(&mut self, worktree: &Path) -> Result<String, RunnerFailure> {
        git_output(worktree, &["rev-parse", "HEAD"]).map(|value| value.trim().to_owned())
    }

    fn changes(&mut self, worktree: &Path) -> Result<GitChanges, RunnerFailure> {
        let output = git_output_bytes(
            worktree,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?;
        parse_git_changes(&output)
    }
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<String, RunnerFailure> {
    let output = git_output_bytes(cwd, args)?;
    String::from_utf8(output).map_err(|_| {
        RunnerFailure::configuration(
            "git_output_invalid",
            "Git returned an invalid local path or revision.",
        )
    })
}

fn git_output_bytes(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, RunnerFailure> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| git_unavailable())?;
    if !output.status.success() || output.stdout.len() > MAX_GIT_OUTPUT_BYTES {
        return Err(RunnerFailure::configuration(
            "git_command_failed",
            "Git could not inspect the connected Execution workspace.",
        ));
    }
    Ok(output.stdout)
}

struct GitChanges {
    paths: Vec<String>,
    truncated: bool,
}

fn parse_git_changes(output: &[u8]) -> Result<GitChanges, RunnerFailure> {
    let mut entries = output
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty());
    let mut paths = Vec::new();
    while let Some(entry) = entries.next() {
        if entry.len() < 4 || entry[2] != b' ' {
            return Err(invalid_git_changes());
        }
        let renamed = matches!(entry[0], b'R' | b'C') || matches!(entry[1], b'R' | b'C');
        let path = std::str::from_utf8(&entry[3..]).map_err(|_| invalid_git_changes())?;
        let normalized = path.replace('\\', "/");
        if normalized.chars().count() > 512 || !safe_relative_path(Path::new(&normalized)) {
            return Err(invalid_git_changes());
        }
        paths.push(normalized);
        if renamed && entries.next().is_none() {
            return Err(invalid_git_changes());
        }
    }
    paths.sort();
    paths.dedup();
    let truncated = paths.len() > MAX_CHANGED_PATHS;
    paths.truncate(MAX_CHANGED_PATHS);
    Ok(GitChanges { paths, truncated })
}

fn invalid_git_changes() -> RunnerFailure {
    RunnerFailure::evidence(
        "git_changes_invalid",
        "Git returned changed paths that could not be represented safely.",
    )
}

fn scope_changed_paths(
    paths: Vec<String>,
    worktree: &Path,
    working_directory: &Path,
) -> Vec<String> {
    let Ok(prefix) = working_directory.strip_prefix(worktree) else {
        return paths;
    };
    if prefix.as_os_str().is_empty() {
        return paths;
    }
    paths
        .into_iter()
        .map(|path| {
            Path::new(&path)
                .strip_prefix(prefix)
                .map_or(path.clone(), |relative| {
                    relative.to_string_lossy().replace('\\', "/")
                })
        })
        .collect()
}

fn git_unavailable() -> RunnerFailure {
    RunnerFailure::configuration(
        "git_unavailable",
        "Git is unavailable for isolated Runner execution.",
    )
}

struct CodexInvocation {
    working_directory: PathBuf,
    profile: Option<String>,
    model: Option<String>,
    prompt: String,
    schema_path: PathBuf,
    events_path: PathBuf,
    handoff_path: PathBuf,
    timeout: Duration,
}

struct PreparedRun {
    run_workspace: PathBuf,
    schema_path: PathBuf,
    events_path: PathBuf,
    handoff_path: PathBuf,
}

enum CodexExit {
    Completed { success: bool },
    TimedOut,
}

trait CodexPort {
    fn execute(&mut self, invocation: &CodexInvocation) -> Result<CodexExit, ()>;
}

struct SystemCodex {
    codex_binary: PathBuf,
}

impl CodexPort for SystemCodex {
    fn execute(&mut self, invocation: &CodexInvocation) -> Result<CodexExit, ()> {
        let events = create_private_output(&invocation.events_path).map_err(|_| ())?;
        let mut command = Command::new(&self.codex_binary);
        command
            .arg("exec")
            .args(["--json", "--color", "never", "--sandbox", "workspace-write"])
            .arg("--cd")
            .arg(&invocation.working_directory)
            .arg("--output-schema")
            .arg(&invocation.schema_path)
            .arg("--output-last-message")
            .arg(&invocation.handoff_path);
        if let Some(profile) = &invocation.profile {
            command.arg("--profile").arg(profile);
        }
        if let Some(model) = &invocation.model {
            command.arg("--model").arg(model);
        }
        let mut child = command
            .arg(&invocation.prompt)
            .stdin(Stdio::null())
            .stdout(Stdio::from(events))
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| ())?;
        let deadline = Instant::now() + invocation.timeout;
        loop {
            match child.try_wait().map_err(|_| ())? {
                Some(status) => {
                    return Ok(CodexExit::Completed {
                        success: status.success(),
                    });
                }
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(CodexExit::TimedOut);
                }
                None => thread::sleep(POLL_INTERVAL),
            }
        }
    }
}

fn create_private_output(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn executes_one_codex_run_in_an_isolated_preserved_worktree() {
        let fixture = Fixture::new();
        let observed = Rc::new(RefCell::new(None));
        let mut runner = CodexRunner::with_ports(
            fixture.runs.clone(),
            Box::new(FakeGit::new(fixture.workspace.clone())),
            Box::new(FakeCodex::successful(Rc::clone(&observed))),
        );

        let result = runner.execute(&fixture.request());

        assert_eq!(result.disposition, CodexRunDisposition::Succeeded);
        assert_eq!(result.resolved_model.model.as_deref(), Some("gpt-run"));
        assert_eq!(result.resolved_model.source, ModelSource::RunOverride);
        let worktree = result.worktree.unwrap();
        assert_eq!(worktree.branch, "gareji/run-run-1");
        assert!(worktree.preserved);
        assert!(worktree.has_uncommitted_changes);
        assert_eq!(worktree.changed_paths, vec!["src/lib.rs".to_owned()]);
        assert_eq!(worktree.base_revision, "base-revision");
        assert_eq!(worktree.final_revision, "final-revision");
        assert_eq!(result.handoff.unwrap().outcome, HandoffOutcome::Completed);

        let invocation = observed.borrow();
        let invocation = invocation.as_ref().unwrap();
        assert_eq!(invocation.model.as_deref(), Some("gpt-run"));
        assert_eq!(invocation.profile.as_deref(), Some("safe-implementation"));
        assert!(invocation.working_directory.starts_with(&fixture.runs));
        assert!(invocation.prompt.contains("BOARD-1"));
    }

    #[test]
    fn system_git_creates_a_real_isolated_worktree() {
        let fixture = Fixture::new();
        fixture.initialize_git_repository();
        let observed = Rc::new(RefCell::new(None));
        let mut runner = CodexRunner::with_ports(
            fixture.runs.clone(),
            Box::new(SystemGit),
            Box::new(FakeCodex::successful(Rc::clone(&observed))),
        );

        let result = runner.execute(&fixture.request());

        assert_eq!(result.disposition, CodexRunDisposition::Succeeded);
        let worktree = result.worktree.unwrap();
        assert!(Path::new(&worktree.root).join(".git").exists());
        assert!(Path::new(&worktree.working_directory).is_dir());
        assert_eq!(worktree.branch, "gareji/run-run-1");
        assert!(worktree.has_uncommitted_changes);
        assert_eq!(worktree.changed_paths, vec!["src/lib.rs".to_owned()]);
        assert!(
            fixture
                .workspace
                .join("agents/implementer/AGENT.md")
                .is_file()
        );
    }

    #[test]
    fn rejects_ineligible_or_unassigned_work_before_creating_a_worktree() {
        let fixture = Fixture::new();
        let creates = Rc::new(RefCell::new(0_u32));
        let mut request = fixture.request();
        request.work_item.state = WorkItemState::Blocked;
        let mut runner = CodexRunner::with_ports(
            fixture.runs.clone(),
            Box::new(FakeGit::counting(
                fixture.workspace.clone(),
                Rc::clone(&creates),
            )),
            Box::new(FakeCodex::never()),
        );

        let result = runner.execute(&request);

        assert_eq!(result.disposition, CodexRunDisposition::Rejected);
        assert_eq!(result.failure.unwrap().code, "work_item_not_eligible");
        assert_eq!(*creates.borrow(), 0);
    }

    #[test]
    fn missing_agent_references_fail_before_worktree_creation() {
        let fixture = Fixture::new();
        fs::remove_file(fixture.workspace.join("agents/implementer/AGENT.md")).unwrap();
        let creates = Rc::new(RefCell::new(0_u32));
        let mut runner = CodexRunner::with_ports(
            fixture.runs.clone(),
            Box::new(FakeGit::counting(
                fixture.workspace.clone(),
                Rc::clone(&creates),
            )),
            Box::new(FakeCodex::never()),
        );

        let result = runner.execute(&fixture.request());

        assert_eq!(result.disposition, CodexRunDisposition::Rejected);
        assert_eq!(
            result.failure.unwrap().code,
            "agent_instruction_unavailable"
        );
        assert_eq!(*creates.borrow(), 0);
    }

    #[test]
    fn timeout_preserves_worktree_and_reports_bounded_failure() {
        let fixture = Fixture::new();
        let mut runner = CodexRunner::with_ports(
            fixture.runs.clone(),
            Box::new(FakeGit::new(fixture.workspace.clone())),
            Box::new(FakeCodex::timed_out()),
        );

        let result = runner.execute(&fixture.request());

        assert_eq!(result.disposition, CodexRunDisposition::TimedOut);
        assert_eq!(result.failure.unwrap().code, "codex_timeout");
        assert!(result.worktree.unwrap().preserved);
    }

    #[test]
    fn invalid_handoff_is_a_protocol_failure_with_recovery_evidence() {
        let fixture = Fixture::new();
        let mut runner = CodexRunner::with_ports(
            fixture.runs.clone(),
            Box::new(FakeGit::new(fixture.workspace.clone())),
            Box::new(FakeCodex::invalid_handoff()),
        );

        let result = runner.execute(&fixture.request());

        assert_eq!(result.disposition, CodexRunDisposition::Failed);
        assert_eq!(result.failure.unwrap().code, "handoff_invalid");
        assert!(result.events_path.is_some());
        assert!(result.worktree.unwrap().preserved);
    }

    #[test]
    fn model_resolution_is_deterministic_and_independent_from_agent_role() {
        let selection = CodexModelSelection {
            run_override: None,
            agent_override: Some("gpt-agent".to_owned()),
            workspace_default: Some("gpt-workspace".to_owned()),
        };
        assert_eq!(selection.resolve().model.as_deref(), Some("gpt-agent"));
        assert_eq!(selection.resolve().source, ModelSource::AgentProfile);

        let inherited = CodexModelSelection::default().resolve();
        assert_eq!(inherited.model, None);
        assert_eq!(inherited.source, ModelSource::CodexDefault);
    }

    struct Fixture {
        root: PathBuf,
        workspace: PathBuf,
        runs: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = env::temp_dir().join(format!(
                "gareji-board-runner-{}-{nonce}",
                std::process::id()
            ));
            let workspace = root.join("repository/project");
            let runs = root.join("runs");
            write_fixture_file(&workspace, "agents/implementer/AGENT.md", "instructions");
            write_fixture_file(
                &workspace,
                ".agents/skills/implement-work/SKILL.md",
                "skill",
            );
            Self {
                root,
                workspace,
                runs,
            }
        }

        fn request(&self) -> CodexRunRequest {
            CodexRunRequest {
                run_id: "run-1".to_owned(),
                execution_workspace: ExecutionWorkspaceConnection {
                    project_id: "board".to_owned(),
                    kind: ExecutionWorkspaceKind::LocalDirectory,
                    location: Some(display_path(&self.workspace)),
                },
                work_item: WorkItemSummary {
                    id: "BOARD-1".to_owned(),
                    project_id: "board".to_owned(),
                    title: "Implement bounded Runner".to_owned(),
                    priority: 1,
                    state: WorkItemState::Todo,
                    approval_requirement: ApprovalRequirement::None,
                    dependency_ids: Vec::new(),
                    agent_profile_id: Some("implementer".to_owned()),
                    required_capabilities: vec!["implementation".to_owned()],
                },
                agent_profile: AgentProfileSummary {
                    id: "implementer".to_owned(),
                    role: "Implementer".to_owned(),
                    capabilities: vec!["implementation".to_owned()],
                    instruction_ref: Some("agents/implementer/AGENT.md".to_owned()),
                    skill_refs: vec!["implement-work".to_owned()],
                },
                model: CodexModelSelection {
                    run_override: Some("gpt-run".to_owned()),
                    agent_override: Some("gpt-agent".to_owned()),
                    workspace_default: Some("gpt-workspace".to_owned()),
                },
                codex_profile: Some("safe-implementation".to_owned()),
                timeout_seconds: 1_200,
            }
        }

        fn initialize_git_repository(&self) {
            let repository = self.workspace.parent().unwrap();
            run_git(repository, &["init", "--initial-branch=main"]);
            run_git(repository, &["add", "."]);
            run_git(
                repository,
                &[
                    "-c",
                    "user.name=Gareji Test",
                    "-c",
                    "user.email=gareji-test@example.invalid",
                    "commit",
                    "-m",
                    "fixture",
                ],
            );
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    struct FakeGit {
        source: PathBuf,
        creates: Rc<RefCell<u32>>,
    }

    impl FakeGit {
        fn new(source: PathBuf) -> Self {
            Self::counting(source, Rc::new(RefCell::new(0)))
        }

        fn counting(source: PathBuf, creates: Rc<RefCell<u32>>) -> Self {
            Self { source, creates }
        }
    }

    impl GitPort for FakeGit {
        fn inspect_workspace(&mut self, workspace: &Path) -> Result<GitWorkspace, RunnerFailure> {
            Ok(GitWorkspace {
                canonical_workspace: fs::canonicalize(workspace).unwrap(),
                repository_root: self.source.parent().unwrap().to_path_buf(),
                relative_workspace: PathBuf::from("project"),
                base_revision: "base-revision".to_owned(),
            })
        }

        fn create_worktree(
            &mut self,
            _repository: &Path,
            destination: &Path,
            _branch: &str,
            _base_revision: &str,
        ) -> Result<(), RunnerFailure> {
            *self.creates.borrow_mut() += 1;
            copy_tree(&self.source, &destination.join("project"));
            Ok(())
        }

        fn revision(&mut self, _worktree: &Path) -> Result<String, RunnerFailure> {
            Ok("final-revision".to_owned())
        }

        fn changes(&mut self, _worktree: &Path) -> Result<GitChanges, RunnerFailure> {
            Ok(GitChanges {
                paths: vec!["project/src/lib.rs".to_owned()],
                truncated: false,
            })
        }
    }

    enum FakeCodexMode {
        Success,
        Timeout,
        InvalidHandoff,
        Never,
    }

    struct FakeCodex {
        mode: FakeCodexMode,
        observed: Option<Rc<RefCell<Option<ObservedInvocation>>>>,
    }

    impl FakeCodex {
        fn successful(observed: Rc<RefCell<Option<ObservedInvocation>>>) -> Self {
            Self {
                mode: FakeCodexMode::Success,
                observed: Some(observed),
            }
        }

        fn timed_out() -> Self {
            Self {
                mode: FakeCodexMode::Timeout,
                observed: None,
            }
        }

        fn invalid_handoff() -> Self {
            Self {
                mode: FakeCodexMode::InvalidHandoff,
                observed: None,
            }
        }

        fn never() -> Self {
            Self {
                mode: FakeCodexMode::Never,
                observed: None,
            }
        }
    }

    impl CodexPort for FakeCodex {
        fn execute(&mut self, invocation: &CodexInvocation) -> Result<CodexExit, ()> {
            if let Some(observed) = &self.observed {
                observed.replace(Some(ObservedInvocation {
                    working_directory: invocation.working_directory.clone(),
                    profile: invocation.profile.clone(),
                    model: invocation.model.clone(),
                    prompt: invocation.prompt.clone(),
                }));
            }
            match self.mode {
                FakeCodexMode::Success => {
                    write_fixture_file(
                        &invocation.working_directory,
                        "src/lib.rs",
                        "runner change",
                    );
                    write_private_file(&invocation.events_path, b"{\"type\":\"done\"}\n").unwrap();
                    write_private_file(
                        &invocation.handoff_path,
                        br#"{"outcome":"completed","summary":"Implemented and verified.","verification":["tests passed"],"risks":[],"next_action":"Review the changes."}"#,
                    )
                    .unwrap();
                    Ok(CodexExit::Completed { success: true })
                }
                FakeCodexMode::Timeout => Ok(CodexExit::TimedOut),
                FakeCodexMode::InvalidHandoff => {
                    write_private_file(&invocation.events_path, b"{}\n").unwrap();
                    write_private_file(&invocation.handoff_path, b"not-json").unwrap();
                    Ok(CodexExit::Completed { success: true })
                }
                FakeCodexMode::Never => panic!("Codex must not be invoked"),
            }
        }
    }

    struct ObservedInvocation {
        working_directory: PathBuf,
        profile: Option<String>,
        model: Option<String>,
        prompt: String,
    }

    fn write_fixture_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn copy_tree(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    fn run_git(repository: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
}
