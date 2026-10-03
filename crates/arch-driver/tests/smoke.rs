//! Opt-in smoke test against the real `claude` CLI: needs a Claude Code login, costs a few cents,
//! never runs in default CI.
//!
//! ```sh
//! ARCH_SMOKE_CLAUDE=1 cargo test -p arch-driver --test smoke -- --ignored
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use arch_driver::tool_layer::config::{self, ToolLayerArgs};
use arch_driver::tool_layer::mcp::TOOLS;
use arch_driver::{ClaudeCode, Context, Driver, Task, TurnEvent};
use arch_facts::{ArchDir, ElementId, Plan, RecordKind, SessionId};

fn enabled() -> bool {
    std::env::var_os("ARCH_SMOKE_CLAUDE").is_some_and(|v| v == "1")
}

#[test]
#[ignore = "real Claude CLI: ARCH_SMOKE_CLAUDE=1 cargo test -p arch-driver --test smoke -- --ignored"]
fn a_real_session_starts_restricts_tools_and_resumes() {
    if !enabled() {
        eprintln!("skipped: set ARCH_SMOKE_CLAUDE=1");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let task = Task {
        prompt: "Reply with exactly: ready".into(),
        cwd: dir.path().to_path_buf(),
        tools: vec!["Read".into(), "Glob".into()],
        model: Some(std::env::var("ARCH_SMOKE_MODEL").unwrap_or_else(|_| "haiku".into())),
        max_turns: Some(3),
        ..Task::default()
    };
    let mut driver = ClaudeCode::new();
    let turn = driver.start(&task, &Context::default()).unwrap();
    let id = turn.session_id().to_string();
    let events: Vec<TurnEvent> = turn.map(Result::unwrap).collect();
    match &events[0] {
        TurnEvent::Started {
            session_id, tools, ..
        } => {
            assert_eq!(session_id, &id, "--session-id is honoured");
            assert_eq!(tools, &["Glob", "Read"], "--tools restricts the set");
        }
        other => panic!("first event: {other:?}"),
    }
    let TurnEvent::Result(first) = events.last().unwrap() else {
        panic!("no result")
    };
    assert!(first.ok, "{first:?}");

    let pointer = driver.pointer(dir.path(), &id);
    let transcript = pointer.transcript_path.expect("a home directory");
    assert!(
        transcript.exists(),
        "transcript at {}",
        transcript.display()
    );

    let again = Task {
        prompt: "Reply with exactly the word you replied last time.".into(),
        ..task
    };
    let second = driver
        .resume(&id, &again, &Context::default())
        .unwrap()
        .finish()
        .unwrap();
    assert!(second.ok, "{second:?}");
    assert_eq!(second.session_id, id, "--resume keeps the session");
    assert!(
        second.text.unwrap_or_default().contains("ready"),
        "the resumed turn remembers"
    );
}

/// The `arch` binary, built for the test: hooks and the MCP server are `arch` subcommands.
fn arch_binary() -> PathBuf {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let ok = Command::new(cargo)
        .args(["build", "-q", "-p", "arch-cli", "--bin", "arch"])
        .current_dir(&workspace)
        .status()
        .unwrap();
    assert!(ok.success(), "cargo build -p arch-cli");
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("target"));
    target.join("debug/arch").canonicalize().unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(ok.success(), "git {args:?}");
}

#[test]
#[ignore = "real Claude CLI: ARCH_SMOKE_CLAUDE=1 cargo test -p arch-driver --test smoke -- --ignored"]
fn a_real_out_of_scope_write_is_denied_and_the_model_reports_why() {
    if !enabled() {
        eprintln!("skipped: set ARCH_SMOKE_CLAUDE=1");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn greet() -> &'static str {\n    \"hello\"\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("NOTES.md"), "notes\n").unwrap();
    std::fs::write(root.join(".gitignore"), ".arch/cache/\n").unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["add", "-A"]);
    git(
        &root,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "init",
        ],
    );

    let session = SessionId::new("s-smoke");
    let mut plan = Plan::new(session.clone(), "notes only");
    plan.add_element("update the notes", "NOTES.md");
    plan.elements[0].files = vec!["NOTES.md".into()];
    let element: ElementId = plan.elements[0].id.clone();
    ArchDir::of_repo(&root)
        .session(&session)
        .write_plan(&plan)
        .unwrap();

    let args = ToolLayerArgs {
        exe: arch_binary(),
        worktree: root.clone(),
        session: session.clone(),
        element: element.clone(),
    };
    let context = config::write_context(&args, None).unwrap();
    let mut allowed: Vec<String> = ["Read", "Edit", "Write"].map(String::from).to_vec();
    allowed.extend(TOOLS.iter().map(|t| t.to_string()));
    let task = Task {
        prompt: "Read src/lib.rs, then use Edit to change \"hello\" to \"hi\" in it. If the edit \
                 is refused, do not retry: reply with the refusal reason, quoted."
            .into(),
        cwd: root.clone(),
        tools: vec!["Read".into(), "Edit".into(), "Write".into()],
        allowed_tools: allowed,
        model: Some(std::env::var("ARCH_SMOKE_MODEL").unwrap_or_else(|_| "haiku".into())),
        max_turns: Some(6),
        ..Task::default()
    };
    let events: Vec<TurnEvent> = ClaudeCode::new()
        .start(&task, &context)
        .unwrap()
        .map(Result::unwrap)
        .collect();

    let TurnEvent::Started { tools, .. } = &events[0] else {
        panic!("first event: {:?}", events[0])
    };
    for tool in TOOLS {
        assert!(
            tools.iter().any(|t| t == tool),
            "arch's MCP server is up: {tools:?}"
        );
    }
    let blocked = events.iter().any(|e| {
        matches!(e, TurnEvent::Hook { event, exit_code: Some(2), output, .. }
            if event == "PreToolUse" && output.contains("outside element E1"))
    });
    assert!(blocked, "arch hook pre blocked the edit: {events:#?}");
    let TurnEvent::Result(result) = events.last().unwrap() else {
        panic!("no result")
    };
    assert!(result.denials.contains(&"Edit".to_string()), "{result:?}");
    let text = result.text.clone().unwrap_or_default();
    assert!(
        text.contains("outside element") || text.contains("NOTES.md"),
        "the model reports the reason: {text}"
    );
    assert!(
        std::fs::read_to_string(root.join("src/lib.rs"))
            .unwrap()
            .contains("hello"),
        "the file is unchanged"
    );
    let records = ArchDir::of_repo(&root)
        .session(&session)
        .read_records()
        .unwrap();
    assert!(
        records
            .iter()
            .any(|r| matches!(&r.kind, RecordKind::Denial { path, .. } if path == Path::new("src/lib.rs"))),
        "a Denial record: {records:?}"
    );
}
