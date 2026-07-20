use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use gareji_board_domain::{
    ExecutionWorkspaceConnection, ExecutionWorkspaceKind, ExecutionWorkspaceSaveRequest,
};
use gareji_board_store::{SqliteBoardStore, default_board_database_path};

const DEMO_MARKER: &str = ".gareji-demo-session";
const DEMO_MARKER_CONTENT: &str = "gareji-board-demo-v1\n";

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
    let mut receipt = prepare_demo_session(&session_root, &fixture_root, request.reset)?;
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
    copy_fixture_if_missing(&fixture_root.join("demo-workspace"), &knowledge_workspace)?;
    copy_fixture_if_missing(
        &fixture_root.join("demo-execution-workspace"),
        &execution_workspace,
    )?;

    let database = session_root.join("board.sqlite3");
    let mut store = SqliteBoardStore::open(&database)?;
    store.seed_sample_if_empty()?;
    store.ensure_builtin_control_graphs()?;
    store.ensure_builtin_portfolio_orchestrations()?;
    store.ensure_builtin_orchestration_blueprints()?;
    connect_sample_execution_workspaces(&mut store, &execution_workspace)?;

    Ok(DemoReceipt {
        session_root: session_root.to_path_buf(),
        database,
        knowledge_workspace,
        execution_workspace,
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

fn find_fixture_root() -> Result<PathBuf> {
    if let Some(explicit) = env::var_os("GAREJI_BOARD_FIXTURES") {
        let path = PathBuf::from(explicit);
        if has_demo_fixtures(&path) {
            return Ok(path);
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
                return Ok(candidate);
            }
        }
    }
    bail!(
        "bundled demo fixtures could not be found; set GAREJI_BOARD_FIXTURES to the examples directory"
    )
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

        let store = SqliteBoardStore::open(&receipt.database).unwrap();
        assert_eq!(store.load_portfolio().unwrap().projects.len(), 3);
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
