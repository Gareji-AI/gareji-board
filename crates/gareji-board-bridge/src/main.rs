use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use clap::{Parser, Subcommand};
use gareji_board_bridge::{
    BOARD_BRIDGE_PROTOCOL_VERSION, BoardBridge, BoardBridgeError, BoardBridgeErrorCode,
    BoardBridgeRequest, BoardBridgeResponse,
};
use gareji_board_core::PortfolioScheduler;
use gareji_board_store::{SqliteBoardStore, default_board_database_path};

mod demo;

const MAX_BRIDGE_LINE_BYTES: usize = 1_048_576;

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
    }
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
