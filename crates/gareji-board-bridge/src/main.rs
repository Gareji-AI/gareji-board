use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use gareji_board_bridge::{
    BOARD_BRIDGE_PROTOCOL_VERSION, BoardBridge, BoardBridgeError, BoardBridgeErrorCode,
    BoardBridgeRequest, BoardBridgeResponse,
};
use gareji_board_store::{SqliteBoardStore, default_board_database_path};

const MAX_BRIDGE_LINE_BYTES: usize = 1_048_576;

#[derive(Debug, Parser)]
#[command(name = "gareji-board", version, about = "Gareji Board local bridge")]
struct Cli {
    #[arg(long, global = true, env = "GAREJI_BOARD_DB")]
    database: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Serve bounded active-work assessments for one parent process.
    Bridge,
    /// Explicitly initialize the disposable sample portfolio when empty.
    SeedSample,
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
    let database = cli.database.unwrap_or_else(default_board_database_path);
    match cli.command {
        Commands::Bridge => run_bridge(&database),
        Commands::SeedSample => {
            let mut store = SqliteBoardStore::open(database)?;
            let created = store.seed_sample_if_empty()?;
            println!("sample_created={created}");
            Ok(())
        }
    }
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
