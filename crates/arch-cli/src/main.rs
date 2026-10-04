//! `arch` · the binary: `init · serve · check · mcp · hook` (ADR 0023).
//!
//! `init`, `check` (milestone 4), `hook` and `mcp` (the tool layer, #46) are live; `serve`
//! reports that it is not yet implemented and exits with status 2. Depends on `arch-api` only.
//!
//! Exit codes of `arch check`: 0 when nothing blocks (clean, or warnings only), 1 when a finding
//! blocks, 2 when the tool itself failed. `arch hook pre` exits 0 to let a call through and 2 to
//! block it, with the reason on stderr; it blocks when it fails, too (fail closed). `arch hook
//! post` blocks nothing: it exits 1 when it fails.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use arch_api::{
    CheckOutput, Finding, InitOptions, InitOutcome, Level, Phase, ToolLayerTarget, Witness,
};
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
    /// Serve the MCP tools (read · check · commit · ask) for one element, over stdio.
    Mcp {
        #[command(flatten)]
        target: Target,
        /// Who the element's commits are co-authored by.
        #[arg(long, default_value = arch_api::CLAUDE_CO_AUTHOR)]
        co_author: String,
    },
    /// Driver hooks (ADR 0012): the hook JSON on stdin.
    Hook {
        /// `pre` (stale-write guard, element-scope veto) or `post` (attribution).
        #[arg(value_enum)]
        phase: HookPhase,
        #[command(flatten)]
        target: Target,
    },
}

/// The element of a session the tool layer works on.
#[derive(clap::Args)]
struct Target {
    /// The session's worktree.
    #[arg(long, default_value = ".")]
    worktree: PathBuf,
    /// The session id.
    #[arg(long)]
    session: String,
    /// The element id.
    #[arg(long)]
    element: String,
}

impl Target {
    fn into_api(self) -> ToolLayerTarget {
        ToolLayerTarget {
            worktree: self.worktree,
            session: self.session,
            element: self.element,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum HookPhase {
    /// Before a tool call.
    Pre,
    /// After a tool call (or its failure).
    Post,
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
        Command::Mcp { target, co_author } => arch_api::mcp(
            &target.into_api(),
            &co_author,
            std::io::stdin().lock(),
            std::io::stdout().lock(),
        )
        .map(|()| 0),
        Command::Hook { phase, target } => return run_hook(phase, target.into_api()),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("arch: {e}");
            ExitCode::from(2)
        }
    }
}

fn run_hook(phase: HookPhase, target: ToolLayerTarget) -> ExitCode {
    let (phase, failed) = match phase {
        HookPhase::Pre => (Phase::Pre, 2),
        HookPhase::Post => (Phase::Post, 1),
    };
    let mut stdin = String::new();
    let outcome = std::io::stdin()
        .read_to_string(&mut stdin)
        .map_err(|e| e.to_string())
        .and_then(|_| arch_api::hook(phase, &target, &stdin).map_err(|e| e.to_string()));
    match outcome {
        Ok(outcome) => {
            eprint!("{}", outcome.stderr);
            ExitCode::from(outcome.exit_code)
        }
        Err(e) => {
            eprintln!("arch hook: {e}");
            ExitCode::from(failed)
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
        "arch check · {} @ {short} · {}",
        output.repo, output.analyzer
    ));
    if let Some(target) = &output.target {
        let pinned = if output.target_pinned {
            " (areas.toml)"
        } else {
            ""
        };
        out.push_str(&format!(" · analyzed for {target}{pinned}"));
    }
    out.push('\n');
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

#[cfg(test)]
mod tests {
    use super::*;

    fn output(target: Option<&str>, target_pinned: bool) -> CheckOutput {
        CheckOutput {
            schema_version: arch_api::CHECK_SCHEMA_VERSION,
            repo: "smallsvc".into(),
            commit: "ff41a0de9ec0bf6878c46b118b025f50cd30c373".into(),
            analyzer: "arch-analyze 0.1.0".into(),
            target: target.map(str::to_string),
            target_pinned,
            degraded: vec![],
            summary: arch_api::Summary {
                blocking: 0,
                warnings: 0,
                allowed: 0,
            },
            findings: vec![],
        }
    }

    #[test]
    fn the_header_says_which_platform_the_facts_were_analysed_for() {
        let first = |o: &CheckOutput| render_check(o).lines().next().unwrap().to_string();
        assert_eq!(
            first(&output(Some("x86_64-unknown-linux-gnu"), true)),
            "arch check · smallsvc @ ff41a0de9ec0 · arch-analyze 0.1.0 · analyzed for x86_64-unknown-linux-gnu (areas.toml)"
        );
        assert_eq!(
            first(&output(Some("aarch64-apple-darwin"), false)),
            "arch check · smallsvc @ ff41a0de9ec0 · arch-analyze 0.1.0 · analyzed for aarch64-apple-darwin"
        );
        // Syntax-level facts read every `#[cfg]` branch: no platform to name.
        assert_eq!(
            first(&output(None, false)),
            "arch check · smallsvc @ ff41a0de9ec0 · arch-analyze 0.1.0"
        );
    }
}
