//! `arch` · the binary: `init · serve · check · mcp · hook` (ADR 0023).
//!
//! `init` and `check` are live (milestone 4); the other commands report that they are not yet
//! implemented and exit with status 2. Depends on `arch-api` only.
//!
//! Exit codes of `arch check`: 0 when nothing blocks (clean, or warnings only), 1 when a finding
//! blocks, 2 when the tool itself failed.

use std::path::PathBuf;
use std::process::ExitCode;

use arch_api::{CheckOutput, Finding, InitOptions, InitOutcome, Level, Witness};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "arch", version, about = "arch · the engine")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write `.arch/` for this repo in one commit, asking no questions (ADR 0006).
    Init {
        /// The repository.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Write the files without committing them.
        #[arg(long)]
        no_commit: bool,
    },
    /// Run the daemon the app and the agents talk to.
    Serve,
    /// CI: facts, findings against `.arch/rules`, exit code, machine-readable output.
    Check {
        /// The repository.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Output format.
        #[arg(long, value_enum, default_value_t = Format::Human)]
        format: Format,
    },
    /// Serve the MCP tools for one session.
    Mcp,
    /// Driver hooks (ADR 0012).
    Hook {
        /// `pre` (stale-write guard, element-scope veto) or `post` (attribution).
        phase: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    /// Lines for a person.
    Human,
    /// The `schemas/check.schema.json` document.
    Json,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Init { path, no_commit } => {
            arch_api::init(&path, &InitOptions { commit: !no_commit }).map(|outcome| {
                print!("{}", render_init(&outcome));
                0
            })
        }
        Command::Check { path, format } => arch_api::check(&path).map(|output| {
            match format {
                Format::Human => print!("{}", render_check(&output)),
                Format::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&output).expect("the output serializes")
                ),
            }
            output.exit_code()
        }),
        Command::Serve => not_yet("serve"),
        Command::Mcp => not_yet("mcp"),
        Command::Hook { .. } => not_yet("hook"),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("arch: {e}");
            ExitCode::from(2)
        }
    }
}

fn not_yet(name: &str) -> Result<u8, arch_api::Error> {
    eprintln!("arch {name}: not implemented yet");
    Ok(2)
}

/// A serde enum value in its kebab-case spelling.
fn word<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn render_init(outcome: &InitOutcome) -> String {
    let mut out = String::new();
    let rule = outcome.areas.rule.as_ref().map(word).unwrap_or_default();
    out.push_str(&format!(
        "arch init · {} areas · {rule}\n",
        outcome.areas.areas.len()
    ));
    for area in &outcome.areas.areas {
        let side = area.side.as_ref().map(word).unwrap_or_default();
        out.push_str(&format!(
            "  {side:<8} {:>2}  {:<28} {}\n",
            area.order.unwrap_or_default(),
            area.name,
            area.paths.join(", ")
        ));
    }
    for path in &outcome.written {
        out.push_str(&format!("wrote {}\n", path.display()));
    }
    match &outcome.commit {
        Some(hash) => out.push_str(&format!("committed {hash}\n")),
        None => out.push_str("not committed (--no-commit)\n"),
    }
    out
}

fn render_check(output: &CheckOutput) -> String {
    let mut out = String::new();
    let short = output.commit.get(..12).unwrap_or(&output.commit);
    out.push_str(&format!(
        "arch check · {} @ {short} · {}\n",
        output.repo, output.analyzer
    ));
    if !output.degraded.is_empty() {
        out.push_str(&format!(
            "note: {} crate(s) analyzed at syntax level; their findings warn and never block\n",
            output.degraded.len()
        ));
    }
    for finding in &output.findings {
        out.push_str(&render_finding(finding));
    }
    let s = output.summary;
    out.push_str(&format!(
        "{} finding(s): {} blocking, {} warning(s), {} allowed\n",
        output.findings.len(),
        s.blocking,
        s.warnings,
        s.allowed
    ));
    out
}

fn render_finding(f: &Finding) -> String {
    let label = match (&f.allowed, f.level) {
        (Some(_), _) => "allowed",
        (None, Level::Block) => "BLOCK",
        (None, Level::Warn) => "warn",
    };
    let at = match &f.witness {
        Witness::Span { file, line, .. } | Witness::Declared { file, line } => {
            format!("{file}:{line}")
        }
        Witness::Tool { tool, .. } => tool.clone(),
    };
    let member = f
        .member
        .as_ref()
        .map(|m| format!(" ·{m}"))
        .unwrap_or_default();
    let mut out = format!(
        "{label:<7} {}\n        {} → {}{member}  ({}, {})\n        at {at}\n",
        f.rule,
        f.site,
        f.target,
        word(&f.kind),
        word(&f.confidence),
    );
    if let Some(allowed) = &f.allowed {
        out.push_str(&format!(
            "        allowed by {}: {}\n",
            allowed.by, allowed.reason
        ));
    }
    out
}
