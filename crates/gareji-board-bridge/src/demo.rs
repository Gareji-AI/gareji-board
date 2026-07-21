use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use gareji_board_domain::{
    ExecutionWorkspaceConnection, ExecutionWorkspaceKind, ExecutionWorkspaceSaveRequest,
    ProjectGraphBinding, ProjectGraphBindingSaveRequest,
};
use gareji_board_store::{SqliteBoardStore, default_board_database_path};
use include_dir::{Dir, include_dir};

const DEMO_MARKER: &str = ".gareji-demo-session";
const DEMO_MARKER_CONTENT: &str = "gareji-board-demo-v1\n";
static EMBEDDED_KNOWLEDGE_FIXTURE: Dir<'_> =
    include_dir!("$CARGO_MANIFEST_DIR/../../examples/demo-workspace");
static EMBEDDED_EXECUTION_FIXTURE: Dir<'_> =
    include_dir!("$CARGO_MANIFEST_DIR/../../examples/demo-execution-workspace");

/// One isolated, resettable local sample launch.
pub struct DemoRequest {
    pub reset: bool,
    pub launch: bool,
    pub app_binary: Option<PathBuf>,
}

/// Paths and optional process identity created for one demo request.
pub struct DemoReceipt {
    pub session_root: PathBuf,
    pub database: PathBuf,
    pub knowledge_workspace: PathBuf,
    pub execution_workspace: PathBuf,
    pub core_database: Option<PathBuf>,
    pub core_binary: Option<PathBuf>,
    pub codex_binary: Option<PathBuf>,
    pub launched_pid: Option<u32>,
    pub launch_log: Option<PathBuf>,
}

/// Prepare the isolated sample and optionally open the desktop Board.
///
/// # Errors
///
/// Returns an error when the fixtures cannot be found, the protected demo
/// directory cannot be prepared, sample storage fails, or the app cannot be
/// launched.
pub fn run(request: &DemoRequest) -> Result<DemoReceipt> {
    let session_root = default_demo_session_root()?;
    let fixture_root = find_fixture_root()?;
    let mut receipt = match fixture_root.as_deref() {
        Some(root) => prepare_demo_session(&session_root, root, request.reset)?,
        None => prepare_embedded_demo_session(&session_root, request.reset)?,
    };
    if let Some(core_binary) = resolve_core_binary()? {
        let core_database = receipt.session_root.join("core.sqlite3");
        prepare_demo_core(&core_binary, &core_database, &receipt.session_root)?;
        receipt.core_database = Some(core_database);
        receipt.core_binary = Some(core_binary);
    }
    receipt.codex_binary = resolve_codex_binary()?;
    if request.launch {
        let (pid, launch_log) = launch_board(&receipt, request.app_binary.as_deref())?;
        receipt.launched_pid = Some(pid);
        receipt.launch_log = Some(launch_log);
    }
    Ok(receipt)
}

fn default_demo_session_root() -> Result<PathBuf> {
    let database = default_board_database_path();
    let parent = database
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .context("the local Board data directory could not be resolved")?;
    Ok(parent.join("demo-session"))
}

fn prepare_demo_session(
    session_root: &Path,
    fixture_root: &Path,
    reset: bool,
) -> Result<DemoReceipt> {
    prepare_demo_session_from_source(
        session_root,
        DemoFixtureSource::Directory(fixture_root),
        reset,
    )
}

fn prepare_embedded_demo_session(session_root: &Path, reset: bool) -> Result<DemoReceipt> {
    prepare_demo_session_from_source(session_root, DemoFixtureSource::Embedded, reset)
}

#[derive(Clone, Copy)]
enum DemoFixtureSource<'a> {
    Directory(&'a Path),
    Embedded,
}

fn prepare_demo_session_from_source(
    session_root: &Path,
    fixture_source: DemoFixtureSource<'_>,
    reset: bool,
) -> Result<DemoReceipt> {
    let marker = session_root.join(DEMO_MARKER);
    if session_root.exists() {
        ensure_demo_marker(&marker)?;
        if reset {
            fs::remove_dir_all(session_root).with_context(|| {
                format!(
                    "the existing demo session could not be reset: {}",
                    session_root.display()
                )
            })?;
        }
    }

    fs::create_dir_all(session_root).with_context(|| {
        format!(
            "the demo session directory could not be created: {}",
            session_root.display()
        )
    })?;
    if !marker.exists() {
        fs::write(&marker, DEMO_MARKER_CONTENT).with_context(|| {
            format!(
                "the demo session marker could not be written: {}",
                marker.display()
            )
        })?;
    }

    let knowledge_workspace = session_root.join("knowledge-workspace");
    let execution_workspace = session_root.join("execution-workspace");
    match fixture_source {
        DemoFixtureSource::Directory(fixture_root) => {
            copy_fixture_if_missing(&fixture_root.join("demo-workspace"), &knowledge_workspace)?;
            copy_fixture_if_missing(
                &fixture_root.join("demo-execution-workspace"),
                &execution_workspace,
            )?;
        }
        DemoFixtureSource::Embedded => {
            copy_embedded_knowledge_fixture_if_missing(&knowledge_workspace)?;
            copy_embedded_execution_fixture_if_missing(&execution_workspace)?;
        }
    }
    initialize_demo_git_repository(&execution_workspace)?;

    let database = session_root.join("board.sqlite3");
    let mut store = SqliteBoardStore::open(&database)?;
    store.seed_sample_if_empty()?;
    store.apply_demo_project_labels()?;
    store.ensure_builtin_control_graphs()?;
    store.ensure_builtin_portfolio_orchestrations()?;
    store.ensure_builtin_orchestration_blueprints()?;
    connect_sample_execution_workspaces(&mut store, &execution_workspace)?;
    bind_sample_control_graphs(&mut store)?;

    Ok(DemoReceipt {
        session_root: session_root.to_path_buf(),
        database,
        knowledge_workspace,
        execution_workspace,
        core_database: None,
        core_binary: None,
        codex_binary: None,
        launched_pid: None,
        launch_log: None,
    })
}

fn ensure_demo_marker(marker: &Path) -> Result<()> {
    let content = fs::read_to_string(marker).with_context(|| {
        format!(
            "refusing to reuse an unrecognized directory as a demo session: {}",
            marker.parent().unwrap_or_else(|| Path::new(".")).display()
        )
    })?;
    if content != DEMO_MARKER_CONTENT {
        bail!(
            "refusing to reuse an unrecognized directory as a demo session: {}",
            marker.parent().unwrap_or_else(|| Path::new(".")).display()
        );
    }
    Ok(())
}

fn copy_fixture_if_missing(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        return Ok(());
    }
    if !source.is_dir() {
        bail!("bundled demo fixture is unavailable: {}", source.display());
    }
    copy_fixture_tree(source, destination)
}

fn copy_fixture_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if file_type.is_symlink() {
            bail!(
                "bundled demo fixtures may not contain symbolic links: {}",
                entry.path().display()
            );
        }
        if file_type.is_dir() {
            copy_fixture_tree(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn copy_embedded_knowledge_fixture_if_missing(destination: &Path) -> Result<()> {
    copy_embedded_fixture_if_missing(&EMBEDDED_KNOWLEDGE_FIXTURE, destination)
}

fn copy_embedded_execution_fixture_if_missing(destination: &Path) -> Result<()> {
    copy_embedded_fixture_if_missing(&EMBEDDED_EXECUTION_FIXTURE, destination)
}

fn copy_embedded_fixture_if_missing(source: &Dir<'_>, destination: &Path) -> Result<()> {
    if destination.exists() {
        return Ok(());
    }
    source.extract(destination).with_context(|| {
        format!(
            "the embedded demo fixture could not be copied to {}",
            destination.display()
        )
    })
}

fn initialize_demo_git_repository(execution_workspace: &Path) -> Result<()> {
    if execution_workspace.join(".git").is_dir() {
        return Ok(());
    }
    run_demo_git(execution_workspace, &["init"])?;
    run_demo_git(execution_workspace, &["add", "--all"])?;
    run_demo_git(
        execution_workspace,
        &[
            "-c",
            "user.name=Gareji Demo",
            "-c",
            "user.email=demo@localhost",
            "commit",
            "-m",
            "chore: initialize Gareji demo workspace",
        ],
    )
}

fn run_demo_git(execution_workspace: &Path, arguments: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .args(arguments)
        .current_dir(execution_workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("Git is required to prepare the isolated demo Runner workspace")?;
    if !status.success() {
        bail!("the isolated demo Runner workspace could not be initialized")
    }
    Ok(())
}

fn connect_sample_execution_workspaces(
    store: &mut SqliteBoardStore,
    execution_workspace: &Path,
) -> Result<()> {
    let canonical = fs::canonicalize(execution_workspace).with_context(|| {
        format!(
            "the copied demo Execution workspace is unavailable: {}",
            execution_workspace.display()
        )
    })?;
    let location = displayable_canonical_path(&canonical)
        .context("the demo Execution workspace path cannot be displayed")?;
    let stored = store.load_execution_workspaces()?;
    for project_id in ["gareji-board", "gareji-core", "zettelkasten-plugin"] {
        let expected = stored
            .iter()
            .find(|connection| connection.project_id == project_id)
            .cloned();
        let target = ExecutionWorkspaceConnection {
            project_id: project_id.to_owned(),
            kind: ExecutionWorkspaceKind::LocalDirectory,
            location: Some(location.clone()),
        };
        store.save_execution_workspace(&ExecutionWorkspaceSaveRequest { expected, target })?;
    }
    Ok(())
}

fn bind_sample_control_graphs(store: &mut SqliteBoardStore) -> Result<()> {
    let existing = store.load_project_graph_bindings()?;
    let bindings = [
        ("gareji-board", "reviewed", "implementation-only"),
        ("gareji-core", "direct", "standard"),
        ("zettelkasten-plugin", "high-risk", "standard"),
    ];
    for (project_id, graph_id, entry_id) in bindings {
        let expected = existing
            .iter()
            .find(|binding| binding.project_id == project_id)
            .cloned();
        let target = ProjectGraphBinding {
            project_id: project_id.to_owned(),
            graph_id: graph_id.to_owned(),
            revision_id: "v1".to_owned(),
            entry_id: entry_id.to_owned(),
        };
        store.save_project_graph_binding(&ProjectGraphBindingSaveRequest { expected, target })?;
    }
    Ok(())
}

fn displayable_canonical_path(path: &Path) -> Option<String> {
    let location = path.to_str()?.trim();
    if location.is_empty() {
        return None;
    }
    #[cfg(windows)]
    {
        if let Some(location) = location.strip_prefix(r"\\?\UNC\") {
            return Some(format!(r"\\{location}"));
        }
        if let Some(location) = location.strip_prefix(r"\\?\") {
            return Some(location.to_owned());
        }
    }
    Some(location.to_owned())
}

fn find_fixture_root() -> Result<Option<PathBuf>> {
    if let Some(explicit) = env::var_os("GAREJI_BOARD_FIXTURES") {
        let path = PathBuf::from(explicit);
        if has_demo_fixtures(&path) {
            return Ok(Some(path));
        }
        bail!(
            "GAREJI_BOARD_FIXTURES does not contain the bundled demo fixtures: {}",
            path.display()
        );
    }

    let mut starts = Vec::new();
    if let Ok(current) = env::current_dir() {
        starts.push(current);
    }
    if let Ok(executable) = env::current_exe()
        && let Some(parent) = executable.parent()
    {
        starts.push(parent.to_path_buf());
    }
    for start in starts {
        for ancestor in start.ancestors() {
            let candidate = ancestor.join("examples");
            if has_demo_fixtures(&candidate) {
                return Ok(Some(candidate));
            }
        }
    }
    Ok(None)
}

fn resolve_core_binary() -> Result<Option<PathBuf>> {
    if let Some(explicit) = env::var_os("GAREJI_CORE_BIN") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Ok(Some(path));
        }
        bail!("the selected Gareji Core binary is unavailable")
    }
    if let Ok(executable) = env::current_exe()
        && let Some(parent) = executable.parent()
    {
        let sibling = parent.join(core_executable_name());
        if sibling.is_file() {
            return Ok(Some(sibling));
        }
    }
    if let Some(path) = env::var_os("PATH")
        && let Some(candidate) = find_executable_on_paths(
            &env::split_paths(&path).collect::<Vec<_>>(),
            &core_executable_name(),
        )
    {
        return Ok(Some(candidate));
    }
    #[cfg(windows)]
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        let installed = PathBuf::from(local_app_data)
            .join("Gareji/bin")
            .join(core_executable_name());
        if installed.is_file() {
            return Ok(Some(installed));
        }
    }
    Ok(None)
}

fn resolve_codex_binary() -> Result<Option<PathBuf>> {
    if let Some(explicit) = env::var_os("GAREJI_CODEX_BIN") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Ok(Some(path));
        }
        bail!("the selected Codex binary is unavailable")
    }
    if let Ok(executable) = env::current_exe()
        && let Some(parent) = executable.parent()
    {
        let sibling = parent.join(codex_executable_name());
        if sibling.is_file() {
            return Ok(Some(sibling));
        }
    }
    #[cfg(windows)]
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        let root = PathBuf::from(local_app_data).join("OpenAI/Codex/bin");
        let mut candidates = fs::read_dir(root)
            .into_iter()
            .flatten()
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path().join(codex_executable_name()))
            .filter(|candidate| candidate.is_file())
            .collect::<Vec<_>>();
        candidates.sort();
        if let Some(candidate) = candidates.pop() {
            return Ok(Some(candidate));
        }
    }
    #[cfg(windows)]
    if let Some(app_data) = env::var_os("APPDATA") {
        let installed = PathBuf::from(app_data)
            .join("npm/node_modules/@openai/codex/node_modules/@openai/codex-win32-x64")
            .join("vendor/x86_64-pc-windows-msvc/bin")
            .join(codex_executable_name());
        if installed.is_file() {
            return Ok(Some(installed));
        }
    }
    if let Some(path) = env::var_os("PATH")
        && let Some(candidate) = find_executable_on_paths(
            &env::split_paths(&path).collect::<Vec<_>>(),
            &codex_executable_name(),
        )
    {
        return Ok(Some(candidate));
    }
    Ok(None)
}

fn find_executable_on_paths(
    paths: &[PathBuf],
    executable_name: &std::ffi::OsStr,
) -> Option<PathBuf> {
    paths
        .iter()
        .map(|directory| directory.join(executable_name))
        .find(|candidate| candidate.is_file())
}

fn prepare_demo_core(core_binary: &Path, database: &Path, session_root: &Path) -> Result<()> {
    let registration_root = session_root.join("core-projects");
    fs::create_dir_all(&registration_root)?;
    for registration in demo_core_registrations() {
        let project_id = registration["project_id"]
            .as_str()
            .context("the demo Core registration is missing its project identity")?;
        let registration_file = registration_root.join(format!("{project_id}.json"));
        fs::write(
            &registration_file,
            serde_json::to_vec_pretty(&registration)?,
        )?;
        let status = Command::new(core_binary)
            .arg("--database")
            .arg(database)
            .args(["project", "register", "--file"])
            .arg(&registration_file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("the isolated demo Core registration could not be started")?;
        if !status.success() {
            bail!("the isolated demo Core registration was rejected")
        }
    }

    let mut child = Command::new(core_binary)
        .arg("--database")
        .arg(database)
        .arg("bridge")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("the isolated demo Core bridge could not be started")?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .context("the isolated demo Core bridge input is unavailable")?;
        for (index, checkpoint) in demo_core_checkpoints().into_iter().enumerate() {
            serde_json::to_writer(
                &mut *stdin,
                &serde_json::json!({
                    "protocol_version": "gareji.core-bridge.v0",
                    "request_id": format!("demo-seed-{}", index + 1),
                    "operation": "record_progress",
                    "payload": { "checkpoint": checkpoint }
                }),
            )?;
            stdin.write_all(b"\n")?;
        }
    }
    let output = child
        .wait_with_output()
        .context("the isolated demo Core bridge did not finish")?;
    if !output.status.success() {
        bail!("the isolated demo Progress Checkpoints could not be recorded")
    }
    let responses = String::from_utf8(output.stdout)
        .context("the isolated demo Core bridge returned invalid text")?;
    let accepted = responses
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<Result<Vec<_>, _>>()?;
    if accepted.len() != 2 || accepted.iter().any(|response| response["status"] != "ok") {
        bail!("the isolated demo Progress Checkpoints were rejected")
    }
    Ok(())
}

fn demo_core_registrations() -> Vec<serde_json::Value> {
    [
        ("gareji-board", "Gareji Board · Sample"),
        ("gareji-core", "Gareji Core · Sample"),
        ("zettelkasten-plugin", "Sample Knowledge Plugin"),
    ]
    .into_iter()
    .map(|(project_id, name)| {
        serde_json::json!({
            "project_id": project_id,
            "name": name,
            "execution_workspace": project_id,
            "context_sources": [],
            "grants": ["write_progress"],
            "sourced_context": [],
            "delivery_targets": []
        })
    })
    .collect()
}

fn demo_core_checkpoints() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "schema_version": "gareji.progress-checkpoint.v0",
            "checkpoint_id": "checkpoint-demo-success",
            "recorded_at": "2026-07-21T10:00:00+09:00",
            "project_id": "gareji-board",
            "work_item_id": "BOARD-1",
            "execution_workspace_id": "gareji-board",
            "source": "manual_cli",
            "actor": { "type": "human", "id": "demo-operator" },
            "outcome": "needs_review",
            "summary": "Portfolio overview is ready for review with verification evidence attached.",
            "changed_paths": ["docs/demo-result.md"],
            "git": null,
            "verification": [{
                "name": "demo verification",
                "status": "passed",
                "evidence_ref": "demo://verification/portfolio-overview"
            }],
            "evidence_refs": ["demo://run/portfolio-overview"],
            "recommended_state": "in_review"
        }),
        serde_json::json!({
            "schema_version": "gareji.progress-checkpoint.v0",
            "checkpoint_id": "checkpoint-demo-blocked",
            "recorded_at": "2026-07-21T09:30:00+09:00",
            "project_id": "zettelkasten-plugin",
            "work_item_id": "ZETTEL-1",
            "execution_workspace_id": "zettelkasten-plugin",
            "source": "runner",
            "actor": { "type": "agent", "id": "researcher" },
            "outcome": "blocked",
            "summary": "Progress paused because the write destination still needs an explicit human choice.",
            "changed_paths": [],
            "git": null,
            "verification": [{
                "name": "destination preflight",
                "status": "failed",
                "evidence_ref": "demo://verification/write-destination"
            }],
            "evidence_refs": ["demo://run/write-destination"],
            "recommended_state": "blocked"
        }),
    ]
}

fn has_demo_fixtures(root: &Path) -> bool {
    root.join("demo-workspace").is_dir() && root.join("demo-execution-workspace").is_dir()
}

fn launch_board(receipt: &DemoReceipt, explicit_app: Option<&Path>) -> Result<(u32, PathBuf)> {
    let launch = resolve_app_launch(explicit_app)?;
    let mut command = match launch {
        AppLaunch::Binary(path) => Command::new(path),
        AppLaunch::Cargo { workspace_root } => {
            let mut command = Command::new("cargo");
            command
                .args(["run", "-p", "gareji-board-app", "--quiet"])
                .current_dir(workspace_root);
            command
        }
    };
    let launch_log = receipt.session_root.join("app-launch.log");
    let output = fs::File::create(&launch_log).with_context(|| {
        format!(
            "the demo app launch log could not be created: {}",
            launch_log.display()
        )
    })?;
    let errors = output.try_clone()?;
    command
        .env("GAREJI_BOARD_DB", &receipt.database)
        .env(
            "GAREJI_DEMO_KNOWLEDGE_WORKSPACE",
            &receipt.knowledge_workspace,
        )
        .env(
            "GAREJI_DEMO_EXECUTION_WORKSPACE",
            &receipt.execution_workspace,
        )
        .stdin(Stdio::null())
        .stdout(Stdio::from(output))
        .stderr(Stdio::from(errors));
    if let (Some(core_binary), Some(core_database)) = (&receipt.core_binary, &receipt.core_database)
    {
        command
            .env("GAREJI_CORE_BIN", core_binary)
            .env("GAREJI_CORE_DB", core_database);
    }
    if let Some(codex_binary) = &receipt.codex_binary {
        command.env("GAREJI_CODEX_BIN", codex_binary);
    }
    let child = command
        .spawn()
        .context("the Gareji Board desktop app could not be launched")?;
    Ok((child.id(), launch_log))
}

enum AppLaunch {
    Binary(PathBuf),
    Cargo { workspace_root: PathBuf },
}

fn resolve_app_launch(explicit_app: Option<&Path>) -> Result<AppLaunch> {
    if let Some(path) = explicit_app {
        if path.is_file() {
            return Ok(AppLaunch::Binary(path.to_path_buf()));
        }
        bail!(
            "the selected Gareji Board app is unavailable: {}",
            path.display()
        );
    }

    if let Ok(executable) = env::current_exe()
        && let Some(parent) = executable.parent()
    {
        let sibling = parent.join(app_executable_name());
        if sibling.is_file() {
            return Ok(AppLaunch::Binary(sibling));
        }
        if let Some(workspace_root) = find_workspace_root(parent) {
            return Ok(AppLaunch::Cargo { workspace_root });
        }
    }
    if let Ok(current) = env::current_dir()
        && let Some(workspace_root) = find_workspace_root(&current)
    {
        return Ok(AppLaunch::Cargo { workspace_root });
    }
    bail!(
        "gareji-board-app could not be found; install it beside gareji-board or set GAREJI_BOARD_APP_BIN"
    )
}

fn app_executable_name() -> OsString {
    let mut name = OsString::from("gareji-board-app");
    name.push(env::consts::EXE_SUFFIX);
    name
}

fn core_executable_name() -> OsString {
    let mut name = OsString::from("gareji-core");
    name.push(env::consts::EXE_SUFFIX);
    name
}

fn codex_executable_name() -> OsString {
    let mut name = OsString::from("codex");
    name.push(env::consts::EXE_SUFFIX);
    name
}

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|ancestor| {
        ancestor
            .join("crates/gareji-board-app/Cargo.toml")
            .is_file()
            .then(|| ancestor.to_path_buf())
    })
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn prepares_an_isolated_demo_with_copied_hidden_skills() {
        let root = temporary_directory("prepared-demo");
        let session = root.join("session");
        let receipt = prepare_demo_session(&session, &fixture_root(), false).unwrap();

        assert!(receipt.database.is_file());
        assert!(
            receipt
                .knowledge_workspace
                .join("4_Project/Gareji-Board/PROJECT.md")
                .is_file()
        );
        assert!(
            receipt
                .execution_workspace
                .join(".agents/skills/review-work-item/SKILL.md")
                .is_file()
        );
        assert!(receipt.execution_workspace.join(".git").is_dir());

        let mut store = SqliteBoardStore::open(&receipt.database).unwrap();
        let projects = store.load_portfolio().unwrap().projects;
        assert_eq!(projects.len(), 3);
        assert_eq!(
            projects
                .iter()
                .find(|project| project.id == "gareji-board")
                .unwrap()
                .name,
            "Gareji Board · Sample"
        );
        assert_eq!(
            projects
                .iter()
                .find(|project| project.id == "gareji-core")
                .unwrap()
                .name,
            "Gareji Core · Sample"
        );
        assert_eq!(
            projects
                .iter()
                .find(|project| project.id == "zettelkasten-plugin")
                .unwrap()
                .name,
            "Sample Knowledge Plugin"
        );
        assert_eq!(store.load_agent_profiles().unwrap().len(), 4);
        let connections = store.load_execution_workspaces().unwrap();
        assert_eq!(connections.len(), 3);
        assert!(connections.iter().all(|connection| {
            connection.kind == ExecutionWorkspaceKind::LocalDirectory
                && connection.location.as_deref()
                    == displayable_canonical_path(
                        &receipt.execution_workspace.canonicalize().unwrap(),
                    )
                    .as_deref()
        }));
        assert_eq!(store.load_project_graph_bindings().unwrap().len(), 3);
        let target = store
            .prepare_agent_loop_execution_target("gareji-core", "CORE-2")
            .expect("the Safe Autopilot demo candidate must be runnable");
        assert_eq!(target.agent_profile_id, "implementer");
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reset_restores_fixture_content_but_a_normal_reopen_preserves_it() {
        let root = temporary_directory("reset-demo");
        let session = root.join("session");
        let first = prepare_demo_session(&session, &fixture_root(), false).unwrap();
        let note = first.knowledge_workspace.join("README.md");
        fs::write(&note, "changed in demo").unwrap();

        prepare_demo_session(&session, &fixture_root(), false).unwrap();
        assert_eq!(fs::read_to_string(&note).unwrap(), "changed in demo");

        prepare_demo_session(&session, &fixture_root(), true).unwrap();
        assert_ne!(fs::read_to_string(&note).unwrap(), "changed in demo");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reset_refuses_to_delete_an_unmarked_directory() {
        let root = temporary_directory("protected-demo");
        let session = root.join("session");
        fs::create_dir_all(&session).unwrap();
        let sentinel = session.join("keep.txt");
        fs::write(&sentinel, "keep").unwrap();

        let error = prepare_demo_session(&session, &fixture_root(), true)
            .err()
            .unwrap();
        assert!(error.to_string().contains("refusing to reuse"));
        assert!(sentinel.is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn embedded_demo_fixtures_include_hidden_skills() {
        let root = temporary_directory("embedded-demo");
        let execution_workspace = root.join("execution-workspace");

        copy_embedded_execution_fixture_if_missing(&execution_workspace).unwrap();

        assert!(
            execution_workspace
                .join(".agents/skills/review-work-item/SKILL.md")
                .is_file()
        );
        assert!(execution_workspace.join("work-items/CORE-2.md").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn core_demo_payloads_cover_success_and_blocked_outcomes() {
        let registrations = demo_core_registrations();
        assert_eq!(registrations.len(), 3);
        assert!(registrations.iter().all(|registration| {
            registration["grants"] == serde_json::json!(["write_progress"])
        }));
        assert_eq!(registrations[0]["name"], "Gareji Board · Sample");
        assert_eq!(registrations[1]["name"], "Gareji Core · Sample");
        assert_eq!(registrations[2]["name"], "Sample Knowledge Plugin");

        let checkpoints = demo_core_checkpoints();
        assert_eq!(checkpoints.len(), 2);
        assert_eq!(checkpoints[0]["project_id"], "gareji-board");
        assert_eq!(checkpoints[0]["outcome"], "needs_review");
        assert_eq!(checkpoints[1]["project_id"], "zettelkasten-plugin");
        assert_eq!(checkpoints[1]["outcome"], "blocked");
    }

    #[test]
    fn resolves_a_native_codex_executable_from_the_launcher_path() {
        let root = temporary_directory("codex-path");
        let native = root.join(codex_executable_name());
        fs::write(&native, b"demo executable").unwrap();

        let resolved =
            find_executable_on_paths(std::slice::from_ref(&root), &codex_executable_name());

        assert_eq!(resolved.as_deref(), Some(native.as_path()));
        fs::remove_dir_all(root).unwrap();
    }

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
    }

    fn temporary_directory(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("gareji-board-{prefix}-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
