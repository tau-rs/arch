//! `arch` · the binary: `init · serve · check · mcp · hook · session` (ADR 0023,
//! arch-design#127).
//!
//! `init`, `check` (milestone 4), `hook` and `mcp` (the tool layer, #46) and `session` (#48) are
//! live; `serve` reports that it is not yet implemented and exits with status 2. Depends on
//! `arch-api` only.
//!
//! Exit codes of `arch check`: 0 when nothing blocks (clean, or warnings only), 1 when a finding
//! blocks, 2 when the tool itself failed. `arch hook pre` exits 0 to let a call through and 2 to
//! block it, with the reason on stderr; it blocks when it fails, too (fail closed). `arch hook
//! post` blocks nothing: it exits 1 when it fails.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use arch_api::{
    CheckOutput, Decision, Finding, InitOptions, InitOutcome, Level, MergeReport, Phase, PrReport,
    SessionConfig, SessionReport, SessionState, ToolLayerTarget, Witness,
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
    /// Plan a change, delegate it, review and merge it (spec §6).
    Session {
        #[command(flatten)]
        options: SessionOptions,
        #[command(subcommand)]
        verb: SessionVerb,
    },
}

/// Where and how sessions run: flags over the `ARCH_*` environment (ADR 0023: Settings later).
#[derive(clap::Args)]
struct SessionOptions {
    /// The repository, or one of its worktrees.
    #[arg(long, global = true, default_value = ".")]
    repo: PathBuf,
    /// The gate's test command [env: ARCH_TEST_COMMAND; default: cargo test --workspace].
    #[arg(long, global = true)]
    test_command: Option<String>,
    /// Where session worktrees go [env: ARCH_WORKTREE_PARENT; default: the repo's parent].
    #[arg(long, global = true)]
    worktree_parent: Option<PathBuf>,
    /// The planner's and elements' model [env: ARCH_MODEL].
    #[arg(long, global = true)]
    model: Option<String>,
    /// The judge's model [env: ARCH_JUDGE_MODEL].
    #[arg(long, global = true)]
    judge_model: Option<String>,
    /// A cap on an element turn's agentic turns [env: ARCH_MAX_TURNS].
    #[arg(long, global = true)]
    max_turns: Option<u32>,
}

impl SessionOptions {
    fn config(&self) -> Result<SessionConfig, arch_api::Error> {
        let mut c = SessionConfig::from_env(|k| std::env::var(k).ok())?;
        if let Some(t) = &self.test_command {
            c.test_command = t.clone();
        }
        if let Some(p) = &self.worktree_parent {
            c.worktree_parent = Some(p.clone());
        }
        if let Some(m) = &self.model {
            c.model = Some(m.clone());
        }
        if let Some(m) = &self.judge_model {
            c.judge_model = Some(m.clone());
        }
        if let Some(n) = self.max_turns {
            c.max_turns = Some(n);
        }
        Ok(c)
    }
}

#[derive(Subcommand)]
enum SessionVerb {
    /// Draft a plan for a change (the planner, or --plan); the draft waits in the cache.
    New {
        /// The change, in a sentence.
        intention: String,
        /// The elements by hand (`[[element]] intention · site · files · depends_on`); skips the
        /// planner.
        #[arg(long)]
        plan: Option<PathBuf>,
        /// Accept the plan and delegate it at once.
        #[arg(long)]
        delegate: bool,
    },
    /// Accept a draft: branch, worktree, session folder; delegate it, or keep it yours.
    Accept {
        /// The session id.
        id: String,
        /// Delegate it to agents rather than keep it as a locked `you` session.
        #[arg(long)]
        delegate: bool,
    },
    /// Run a delegated session on from where it stands (after a crash or a restart).
    Run {
        /// The session id.
        id: String,
    },
    /// Where a session stands.
    Status {
        /// The session id.
        id: String,
        /// Output format.
        #[arg(long, value_enum, default_value_t = Format::Human)]
        format: Format,
    },
    /// Answer the session's open questions, in order, and run on.
    Answer {
        /// The session id.
        id: String,
        /// One answer per question.
        #[arg(required = true)]
        answers: Vec<String>,
    },
    /// Settle a failed gate (four doors) or a deviation, and run on.
    Decide {
        /// The session id.
        id: String,
        #[command(subcommand)]
        decision: DecideVerb,
    },
    /// Push the branch and open the PR, described from the plan and the thread.
    Pr {
        /// The session id.
        id: String,
        /// The branch to merge into; the repository's current branch by default.
        #[arg(long)]
        base: Option<String>,
    },
    /// Merge the PR with the repo's strategy, archive the session to refs/notes/arch, remove its
    /// worktree and branch.
    Merge {
        /// The session id.
        id: String,
        /// The strategy, when the repo allows several.
        #[arg(long, value_enum)]
        strategy: Option<StrategyArg>,
    },
}

#[derive(Subcommand)]
enum DecideVerb {
    /// Gate: one more fix round.
    OneMore {
        /// What to tell the failing elements' agents.
        #[arg(long)]
        hint: Option<String>,
    },
    /// Gate: accept as is; an Override record keeps the reason.
    Accept {
        /// Why.
        #[arg(long, required = true)]
        reason: String,
        /// Who; git's user.name by default.
        #[arg(long)]
        by: Option<String>,
    },
    /// Deviation: back on the plan.
    BackOnPlan,
    /// Deviation: the denied paths join the element's files.
    UpdatePlan,
    /// Deviation: not part of this change.
    NotThisChange,
}

#[derive(Clone, Copy, ValueEnum)]
enum StrategyArg {
    /// A merge commit.
    Merge,
    /// One squashed commit.
    Squash,
    /// Rebased onto the base.
    Rebase,
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
        Command::Session { options, verb } => run_session(&options, verb),
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

fn run_session(options: &SessionOptions, verb: SessionVerb) -> Result<u8, arch_api::Error> {
    let config = options.config()?;
    let repo = &options.repo;
    match verb {
        SessionVerb::New {
            intention,
            plan,
            delegate,
        } => {
            let report =
                arch_api::session_new(repo, &intention, plan.as_deref(), delegate, &config)?;
            print!("{}", render_plan(&report));
            if delegate {
                print!("{}", render_run(&report, true));
            } else {
                println!(
                    "draft in the cache · arch session accept {} --delegate",
                    report.id
                );
            }
        }
        SessionVerb::Accept { id, delegate } => {
            let report = arch_api::session_accept(repo, &id, delegate, &config)?;
            print!("{}", render_run(&report, true));
        }
        SessionVerb::Run { id } => {
            print!(
                "{}",
                render_run(&arch_api::session_run(repo, &id, &config)?, false)
            );
        }
        SessionVerb::Status { id, format } => {
            let report = arch_api::session_status(repo, &id)?;
            match format {
                Format::Human => print!("{}", render_status(&report)),
                Format::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&report).expect("the report serializes")
                ),
            }
        }
        SessionVerb::Answer { id, answers } => {
            let report = arch_api::session_answer(repo, &id, answers, &config)?;
            print!("{}", render_run(&report, false));
        }
        SessionVerb::Decide { id, decision } => {
            let decision = match decision {
                DecideVerb::OneMore { hint } => Decision::OneMore { hint },
                DecideVerb::Accept { reason, by } => Decision::AcceptAsIs { reason, by },
                DecideVerb::BackOnPlan => Decision::BackOnPlan,
                DecideVerb::UpdatePlan => Decision::UpdatePlan,
                DecideVerb::NotThisChange => Decision::NotThisChange,
            };
            let report = arch_api::session_decide(repo, &id, decision, &config)?;
            print!("{}", render_run(&report, false));
        }
        SessionVerb::Pr { id, base } => {
            print!(
                "{}",
                render_pr(&arch_api::session_pr(repo, &id, base.as_deref(), &config)?)
            );
        }
        SessionVerb::Merge { id, strategy } => {
            let strategy = strategy.map(|s| match s {
                StrategyArg::Merge => arch_api::Strategy::Merge,
                StrategyArg::Squash => arch_api::Strategy::Squash,
                StrategyArg::Rebase => arch_api::Strategy::Rebase,
            });
            print!(
                "{}",
                render_merge(&arch_api::session_merge(repo, &id, strategy, &config)?)
            );
        }
    }
    Ok(0)
}

fn render_plan(r: &SessionReport) -> String {
    let mut out = format!(
        "plan {} · {} element(s), {} group(s)\n",
        r.id,
        r.plan.elements.len(),
        r.plan.groups.len()
    );
    for e in &r.plan.elements {
        out.push_str(&format!(
            "  {} {}  {:<32} {}\n",
            e.label, e.id, e.intention, e.site
        ));
    }
    out
}

/// What a run left: the accept line, the elements done, the gates, then what it waits on.
fn render_run(r: &SessionReport, accepted: bool) -> String {
    let mut out = String::new();
    if accepted && let (Some(branch), Some(worktree)) = (&r.branch, &r.worktree) {
        out.push_str(&format!(
            "accepted · branch {branch} · worktree {}\n",
            worktree.display()
        ));
    }
    if r.state == SessionState::Yours {
        out.push_str("yours · a locked `you` session; the plan is in the worktree\n");
        return out;
    }
    let done: Vec<&str> = r
        .plan
        .elements
        .iter()
        .filter(|e| word(&e.state) == "done")
        .map(|e| e.label.as_str())
        .collect();
    if !r.plan.groups.is_empty() {
        out.push_str(&format!(
            "running group {}/{} · {} done\n",
            (r.group as usize + 1).min(r.plan.groups.len()),
            r.plan.groups.len(),
            if done.is_empty() {
                "none".into()
            } else {
                done.join(" ")
            }
        ));
    }
    for g in &r.gates {
        out.push_str(g);
        out.push('\n');
    }
    out.push_str(&render_next(r));
    out
}

fn render_status(r: &SessionReport) -> String {
    let mut out = format!("session {} · {} · {}\n", r.id, r.name, word(&r.state));
    if let (Some(branch), Some(worktree)) = (&r.branch, &r.worktree) {
        out.push_str(&format!(
            "branch {branch} · worktree {}\n",
            worktree.display()
        ));
    }
    for e in &r.plan.elements {
        out.push_str(&format!(
            "  {} {}  {:<8} {:<32} {}\n",
            e.label,
            e.id,
            word(&e.state),
            e.intention,
            e.site
        ));
    }
    for g in &r.gates {
        out.push_str(g);
        out.push('\n');
    }
    out.push_str(&render_next(r));
    out
}

/// The state's last line: what the session waits on, and the command that moves it.
fn render_next(r: &SessionReport) -> String {
    let id = &r.id;
    let questions = || {
        r.questions
            .iter()
            .map(|q| {
                let mut s = format!("  ? {}\n", q.text);
                for o in &q.options {
                    s.push_str(&format!("    - {o}\n"));
                }
                s
            })
            .collect::<String>()
    };
    match r.state {
        SessionState::Planning => {
            format!("draft in the cache · arch session accept {id} --delegate\n")
        }
        SessionState::Done => format!("done · arch session pr {id} to open the PR\n"),
        SessionState::Asks => format!(
            "asks\n{}arch session answer {id} \"<answer>\" …\n",
            questions()
        ),
        SessionState::GateFailed => format!(
            "gate failed\n{}arch session decide {id} one-more [--hint …] | accept --reason …\n",
            questions()
        ),
        SessionState::Deviation => format!(
            "deviation · denied: {}\narch session decide {id} back-on-plan | update-plan | not-this-change\n",
            r.denied
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        SessionState::InReview => format!("in review · arch session merge {id} once approved\n"),
        SessionState::Archived => match &r.archived_on {
            Some(sha) => format!("archived · refs/notes/arch on {sha}\n"),
            None => format!("merged · archiving; arch session merge {id} finishes it\n"),
        },
        other => format!("{} · arch session run {id}\n", word(&other)),
    }
}

fn render_pr(r: &PrReport) -> String {
    format!(
        "{} #{} · {} · {}\n",
        r.word,
        r.number,
        r.url,
        if r.created {
            "opened, in review"
        } else {
            "already open, in review"
        }
    )
}

fn render_merge(r: &MergeReport) -> String {
    let short = r.sha.get(..12).unwrap_or(&r.sha);
    let mut out = format!(
        "merged {} #{} ({}) as {short} · archived to refs/notes/arch ({} files)\n",
        r.word,
        r.number,
        r.strategy,
        r.files.len()
    );
    match &r.worktree {
        Some(w) => out.push_str(&format!(
            "removed worktree {} and branch {}\n",
            w.display(),
            r.branch
        )),
        None => out.push_str(&format!("removed branch {}\n", r.branch)),
    }
    out
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
