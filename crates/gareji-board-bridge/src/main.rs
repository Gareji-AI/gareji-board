use std::fs;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use gareji_board_bridge::{
    BOARD_BRIDGE_PROTOCOL_VERSION, BoardBridge, BoardBridgeError, BoardBridgeErrorCode,
    BoardBridgeRequest, BoardBridgeResponse,
};
use gareji_board_core::{
    BlueprintEditPlan, ControlGraphEditPlan, DefinitionEditor, PortfolioScheduler,
};
use gareji_board_store::{SqliteBoardStore, default_board_database_path};
use serde::Serialize;
use serde::de::DeserializeOwned;

mod demo;

const MAX_BRIDGE_LINE_BYTES: usize = 1_048_576;
const MAX_DEFINITION_PLAN_BYTES: u64 = 1_048_576;

#[derive(Debug, Parser)]
#[command(name = "gareji-board", version, about = "Gareji Board local bridge")]
struct Cli {
    /// Override the normal Board database; demo always uses its isolated session.
    #[arg(long, global = true, env = "GAREJI_BOARD_DB")]
    database: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Open an isolated, resettable sample without using the normal Board database.
    Demo {
        /// Restore the demo session to its bundled initial state before opening.
        #[arg(long)]
        reset: bool,
        /// Prepare and validate the sample without opening the desktop app.
        #[arg(long)]
        prepare_only: bool,
        /// Override the desktop app binary used by this launcher.
        #[arg(long, env = "GAREJI_BOARD_APP_BIN", hide = true)]
        app: Option<PathBuf>,
    },
    /// Serve bounded active-work assessments for one parent process.
    Bridge,
    /// Explicitly initialize the disposable sample portfolio when empty.
    SeedSample,
    /// Advance every due Portfolio schedule once, then exit.
    #[command(name = "portfolio-tick-due")]
    PortfolioTickDue {
        /// Override the clock for deterministic local verification.
        #[arg(long, hide = true)]
        now_epoch_seconds: Option<i64>,
    },
    /// Keep Portfolio schedules running without opening the desktop UI.
    #[command(name = "portfolio-daemon")]
    PortfolioDaemon {
        /// How often the daemon checks for due immutable schedules.
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=3600))]
        poll_interval_seconds: u64,
        /// Stop after this many passes; intended for local verification.
        #[arg(long, hide = true, value_parser = clap::value_parser!(u64).range(1..))]
        max_passes: Option<u64>,
    },
    /// Inspect or atomically publish immutable Control Graph revisions.
    #[command(name = "control-graph")]
    ControlGraph {
        #[command(subcommand)]
        command: ControlGraphCommands,
    },
    /// Inspect or atomically publish immutable Orchestration Blueprints.
    Blueprint {
        #[command(subcommand)]
        command: BlueprintCommands,
    },
}

#[derive(Debug, Subcommand)]
enum ControlGraphCommands {
    /// Print every stored Control Graph revision as JSON.
    List,
    /// Print one exact Control Graph revision as JSON.
    Show {
        #[arg(long)]
        graph_id: String,
        #[arg(long)]
        revision_id: String,
    },
    /// Apply one JSON edit plan and publish its new immutable revision.
    Apply {
        /// JSON plan path, or '-' to read from standard input.
        #[arg(long)]
        file: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum BlueprintCommands {
    /// Print every stored Blueprint revision as JSON.
    List,
    /// Print one exact Blueprint revision as JSON.
    Show {
        #[arg(long)]
        blueprint_id: String,
        #[arg(long)]
        revision_id: String,
    },
    /// Apply one JSON edit plan and publish its new immutable revision.
    Apply {
        /// JSON plan path, or '-' to read from standard input.
        #[arg(long)]
        file: PathBuf,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("gareji-board: {error:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let Cli { database, command } = cli;
    if let Commands::Demo {
        reset,
        prepare_only,
        app,
    } = command
    {
        let receipt = demo::run(&demo::DemoRequest {
            reset,
            launch: !prepare_only,
            app_binary: app,
        })?;
        println!("demo_session={}", receipt.session_root.display());
        println!("demo_database={}", receipt.database.display());
        println!(
            "demo_knowledge_workspace={}",
            receipt.knowledge_workspace.display()
        );
        println!(
            "demo_execution_workspace={}",
            receipt.execution_workspace.display()
        );
        if let Some(pid) = receipt.launched_pid {
            println!("demo_app_pid={pid}");
            if let Some(log) = receipt.launch_log {
                println!("demo_app_log={}", log.display());
            }
        } else {
            println!("demo_app=not_launched");
        }
        return Ok(());
    }

    let database = database.unwrap_or_else(default_board_database_path);
    match command {
        Commands::Demo { .. } => unreachable!("demo returned before database selection"),
        Commands::Bridge => run_bridge(&database),
        Commands::SeedSample => {
            let mut store = SqliteBoardStore::open(database)?;
            let created = store.seed_sample_if_empty()?;
            let definitions_created = store.ensure_builtin_control_graphs()?
                + store.ensure_builtin_portfolio_orchestrations()?
                + store.ensure_builtin_orchestration_blueprints()?;
            println!("sample_created={created} definitions_created={definitions_created}");
            Ok(())
        }
        Commands::PortfolioTickDue { now_epoch_seconds } => {
            let now_epoch_seconds = now_epoch_seconds.map_or_else(current_epoch_seconds, Ok)?;
            let mut scheduler = PortfolioScheduler::open(database)?;
            let report = scheduler.tick_due_once(now_epoch_seconds)?;
            println!("{}", serde_json::to_string(&report)?);
            Ok(())
        }
        Commands::PortfolioDaemon {
            poll_interval_seconds,
            max_passes,
        } => run_portfolio_daemon(&database, poll_interval_seconds, max_passes),
        Commands::ControlGraph { command } => run_control_graph(&database, command),
        Commands::Blueprint { command } => run_blueprint(&database, command),
    }
}

fn run_portfolio_daemon(
    database: &Path,
    poll_interval_seconds: u64,
    max_passes: Option<u64>,
) -> Result<()> {
    let mut scheduler = PortfolioScheduler::open(database)?;
    let mut completed_passes = 0_u64;
    loop {
        let report = scheduler.tick_due_once(current_epoch_seconds()?)?;
        print_json(&report)?;
        completed_passes += 1;
        if max_passes.is_some_and(|limit| completed_passes >= limit) {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(poll_interval_seconds));
    }
}

fn run_control_graph(database: &Path, command: ControlGraphCommands) -> Result<()> {
    let mut editor = DefinitionEditor::open(database)?;
    match command {
        ControlGraphCommands::List => print_json(&editor.control_graphs()?),
        ControlGraphCommands::Show {
            graph_id,
            revision_id,
        } => print_json(&editor.control_graph(&graph_id, &revision_id)?),
        ControlGraphCommands::Apply { file } => {
            let plan = read_definition_plan::<ControlGraphEditPlan>(&file)?;
            print_json(&editor.apply_control_graph(&plan)?)
        }
    }
}

fn run_blueprint(database: &Path, command: BlueprintCommands) -> Result<()> {
    let mut editor = DefinitionEditor::open(database)?;
    match command {
        BlueprintCommands::List => print_json(&editor.blueprints()?),
        BlueprintCommands::Show {
            blueprint_id,
            revision_id,
        } => print_json(&editor.blueprint(&blueprint_id, &revision_id)?),
        BlueprintCommands::Apply { file } => {
            let plan = read_definition_plan::<BlueprintEditPlan>(&file)?;
            print_json(&editor.apply_blueprint(&plan)?)
        }
    }
}

fn read_definition_plan<T: DeserializeOwned>(file: &Path) -> Result<T> {
    let bytes = if file == Path::new("-") {
        let mut bytes = Vec::new();
        io::stdin()
            .take(MAX_DEFINITION_PLAN_BYTES + 1)
            .read_to_end(&mut bytes)?;
        bytes
    } else {
        let metadata = fs::metadata(file)
            .with_context(|| format!("cannot inspect definition plan {}", file.display()))?;
        if metadata.len() > MAX_DEFINITION_PLAN_BYTES {
            bail!("definition plan exceeds the 1 MiB limit");
        }
        fs::read(file).with_context(|| format!("cannot read definition plan {}", file.display()))?
    };
    if u64::try_from(bytes.len())? > MAX_DEFINITION_PLAN_BYTES {
        bail!("definition plan exceeds the 1 MiB limit");
    }
    serde_json::from_slice(&bytes).context("definition plan is not valid JSON")
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn current_epoch_seconds() -> Result<i64> {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    Ok(i64::try_from(seconds)?)
}

fn run_bridge(database: &PathBuf) -> Result<()> {
    let bridge = BoardBridge::open(database)?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    let mut line = Vec::new();

    while read_bounded_line(&mut reader, &mut line)? {
        let response = match serde_json::from_slice::<BoardBridgeRequest>(&line) {
            Ok(request) => bridge.handle(request),
            Err(_) => BoardBridgeResponse::Error {
                protocol_version: BOARD_BRIDGE_PROTOCOL_VERSION.to_owned(),
                request_id: String::new(),
                error: BoardBridgeError {
                    code: BoardBridgeErrorCode::InvalidRequest,
                    message: "invalid Board bridge request".to_owned(),
                },
            },
        };
        serde_json::to_writer(&mut writer, &response)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    Ok(())
}

fn read_bounded_line(reader: &mut impl BufRead, output: &mut Vec<u8>) -> io::Result<bool> {
    output.clear();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(!output.is_empty());
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |position| position + 1);
        let payload_len = newline.unwrap_or(consumed);
        if output.len().saturating_add(payload_len) > MAX_BRIDGE_LINE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Board bridge request exceeds maximum size",
            ));
        }
        output.extend_from_slice(&available[..payload_len]);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn one_shot_scheduler_command_is_parseable() {
        let cli = Cli::try_parse_from(["gareji-board", "portfolio-tick-due"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::PortfolioTickDue {
                now_epoch_seconds: None
            }
        ));
    }

    #[test]
    fn headless_daemon_command_is_parseable() {
        let cli = Cli::try_parse_from([
            "gareji-board",
            "portfolio-daemon",
            "--poll-interval-seconds",
            "15",
            "--max-passes",
            "2",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::PortfolioDaemon {
                poll_interval_seconds: 15,
                max_passes: Some(2),
            }
        ));
    }

    #[test]
    fn control_graph_apply_command_is_parseable() {
        let cli = Cli::try_parse_from([
            "gareji-board",
            "control-graph",
            "apply",
            "--file",
            "graph-edit.json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::ControlGraph {
                command: ControlGraphCommands::Apply { file }
            } if file == Path::new("graph-edit.json")
        ));
    }

    #[test]
    fn blueprint_show_command_is_parseable() {
        let cli = Cli::try_parse_from([
            "gareji-board",
            "blueprint",
            "show",
            "--blueprint-id",
            "evidence-first",
            "--revision-id",
            "v1",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Blueprint {
                command: BlueprintCommands::Show {
                    blueprint_id,
                    revision_id,
                }
            } if blueprint_id == "evidence-first" && revision_id == "v1"
        ));
    }

    #[test]
    fn isolated_demo_command_is_parseable() {
        let cli =
            Cli::try_parse_from(["gareji-board", "demo", "--reset", "--prepare-only"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Demo {
                reset: true,
                prepare_only: true,
                app: None,
            }
        ));
    }

    #[test]
    fn bounded_reader_rejects_an_oversized_request() {
        let mut input = Cursor::new(vec![b'x'; MAX_BRIDGE_LINE_BYTES + 1]);
        let mut output = Vec::new();
        assert_eq!(
            read_bounded_line(&mut input, &mut output)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}
