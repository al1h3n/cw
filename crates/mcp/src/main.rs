//! `cowatcher-mcp` — a Model Context Protocol server for Co-watcher.
//!
//! It exposes the Console's typed, closed set of remote actions (see one another PC's screen, list
//! and launch published apps, block programs, lock/power a PC, run an exam lockdown, set a wallpaper,
//! record a screen) as MCP tools, so an AI agent can operate a classroom **precisely** (every tool is
//! typed and validated) and **safely** (there is no arbitrary-command tool, and a standing operator
//! skill is sent once at connect via the `instructions` field, not repeated per prompt).
//!
//! Transport is JSON-RPC 2.0 over stdio — the shape every MCP client speaks — so it plugs into an AI
//! client with a one-line command entry. It reads the same data directory the Console uses, acting as
//! that console; only PCs already paired in the Console can be reached.

mod fleet;
mod server;
mod tools;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::fleet::Fleet;
use crate::server::Server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|a| a == "--version") {
        println!("cowatcher-mcp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let dir = data_dir(&args);
    eprintln!(
        "cowatcher-mcp: using console state in {} (set --data-dir or COWATCHER_DIR to change)",
        dir.display()
    );
    let fleet = Arc::new(Fleet::load(&dir)?);
    eprintln!(
        "cowatcher-mcp: console {} with {} paired device(s); ready on stdio",
        fleet.console_id(),
        fleet.devices().len()
    );

    let server = Server::new(fleet);
    serve_stdio(&server).await
}

/// Reads newline-delimited JSON-RPC from stdin and writes responses to stdout until stdin closes.
async fn serve_stdio(server: &Server) -> anyhow::Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server.handle_line(&line).await {
            stdout.write_all(response.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}

/// Resolves the Console data directory: `--data-dir <path>`, else `COWATCHER_DIR`, else the Console's
/// default (`%LOCALAPPDATA%\co-watcher\console`, matching `crates/console`).
fn data_dir(args: &[String]) -> PathBuf {
    if let Some(pos) = args.iter().position(|a| a == "--data-dir")
        && let Some(path) = args.get(pos + 1)
    {
        return PathBuf::from(path);
    }
    if let Some(dir) = std::env::var_os("COWATCHER_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("co-watcher").join("console")
}

/// Prints command-line usage.
fn print_help() {
    println!(
        "cowatcher-mcp — Co-watcher Model Context Protocol server\n\n\
         Speaks MCP (JSON-RPC 2.0) over stdio; add it to an AI client as the command to run.\n\n\
         USAGE:\n    cowatcher-mcp [--data-dir <path>]\n\n\
         OPTIONS:\n    \
         --data-dir <path>   Console state directory (default: COWATCHER_DIR or the Console default)\n    \
         --version           Print the version and exit\n    \
         -h, --help          Show this help\n\n\
         The server reaches only PCs already paired in that Console."
    );
}
