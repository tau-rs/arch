//! `arch` · the binary: `init · serve · check · mcp · hook` (ADR 0023).
//!
//! Milestone 1 ships the command surface only; each command reports that it is not yet
//! implemented and exits with status 2.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "arch", version, about = "arch · the engine")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write `.arch/` for this repo in one commit, asking no questions (ADR 0006).
    Init,
    /// Run the daemon the app and the agents talk to.
    Serve,
    /// CI: facts for the diff, findings, exit code, machine-readable output.
    Check,
    /// Serve the MCP tools for one session.
    Mcp,
    /// Driver hooks (ADR 0012).
    Hook {
        /// `pre` (stale-write guard, element-scope veto) or `post` (attribution).
        phase: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let name = match cli.command {
        Command::Init => "init",
        Command::Serve => "serve",
        Command::Check => "check",
        Command::Mcp => "mcp",
        Command::Hook { .. } => "hook",
    };
    eprintln!("arch {name}: not implemented in milestone 1");
    ExitCode::from(2)
}
