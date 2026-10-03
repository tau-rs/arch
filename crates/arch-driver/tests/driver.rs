//! The claude-code adapter (argv, process handling against a stub binary) and the replay double.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use arch_driver::{ClaudeCode, Context, Driver, DriverError, ReplayDriver, Task, TurnEvent};
use pretty_assertions::assert_eq;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(fixtures().join(format!("{name}.jsonl"))).unwrap()
}

fn strings(args: Vec<OsString>) -> Vec<String> {
    args.into_iter().map(|a| a.into_string().unwrap()).collect()
}

#[test]
fn argv_is_the_f1_command_line_with_session_id() {
    let task = Task {
        prompt: "ignored: the prompt goes on stdin".into(),
        cwd: "/w".into(),
        tools: vec!["Read".into(), "Edit".into(), "Write".into()],
        allowed_tools: vec!["Read".into(), "mcp__arch__commit".into()],
        output_schema: Some(serde_json::json!({"type": "object"})),
        model: Some("haiku".into()),
        max_turns: Some(30),
    };
    let context = Context {
        pack: Some("/c/pack.md".into()),
        settings: Some("/c/settings.json".into()),
        mcp_config: Some("/c/mcp.json".into()),
    };
    let argv = strings(ClaudeCode::new().args(false, "u-1", &task, &context));
    assert_eq!(
        argv,
        [
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-hook-events",
            "--setting-sources",
            "",
            "--strict-mcp-config",
            "--session-id",
            "u-1",
            "--model",
            "haiku",
            "--settings",
            "/c/settings.json",
            "--mcp-config",
            "/c/mcp.json",
            "--append-system-prompt-file",
            "/c/pack.md",
            "--tools",
            "Read,Edit,Write",
            "--allowedTools",
            "Read,mcp__arch__commit",
            "--json-schema",
            r#"{"type":"object"}"#,
            "--max-turns",
            "30",
            "--permission-mode",
            "acceptEdits",
            "--permission-prompts",
            "none",
        ]
    );
    assert!(!argv.iter().any(|a| a == "--bare"), "FINDINGS F-1");
    assert!(
        !argv.iter().any(|a| a == "--no-session-persistence"),
        "arch-design#33"
    );
}

#[test]
fn resume_passes_the_session_and_omits_what_is_unset() {
    let argv = strings(ClaudeCode::new().args(true, "u-1", &Task::default(), &Context::default()));
    assert_eq!(&argv[8..10], ["--resume", "u-1"]);
    for absent in [
        "--session-id",
        "--tools",
        "--allowedTools",
        "--settings",
        "--json-schema",
        "--model",
    ] {
        assert!(!argv.iter().any(|a| a == absent), "{absent} only when set");
    }
}

/// A stand-in for `claude`: saves argv, cwd and stdin, then plays `$STUB_PLAY` or fails.
fn stub(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("claude-stub");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{d}/argv\"\npwd > \"{d}/cwd\"\ncat > \"{d}/stdin\"\n{body}\n",
            d = dir.display()
        ),
    )
    .unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(&path)
        .status()
        .unwrap();
    path
}

#[test]
fn the_adapter_spawns_in_the_worktree_feeds_stdin_and_streams_events() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = tempfile::tempdir().unwrap();
    let play = dir.path().join("play.jsonl");
    std::fs::write(&play, fixture("tools")).unwrap();
    let program = stub(dir.path(), &format!("cat \"{}\"", play.display()));
    let task = Task {
        prompt: "change hello to hi".into(),
        cwd: worktree.path().to_path_buf(),
        ..Task::default()
    };
    let mut driver = ClaudeCode::with_program(&program);
    let turn = driver.start(&task, &Context::default()).unwrap();
    let id = turn.session_id().to_string();
    assert_eq!(id.len(), 36, "arch chooses a uuid");
    let events: Vec<TurnEvent> = turn.map(Result::unwrap).collect();
    assert!(matches!(events.first(), Some(TurnEvent::Started { .. })));
    assert!(matches!(events.last(), Some(TurnEvent::Result(r)) if r.ok));

    let read = |f: &str| std::fs::read_to_string(dir.path().join(f)).unwrap();
    assert_eq!(read("stdin"), "change hello to hi");
    assert_eq!(
        Path::new(read("cwd").trim()).canonicalize().unwrap(),
        worktree.path().canonicalize().unwrap()
    );
    assert!(
        read("argv").lines().any(|l| l == id),
        "the chosen id is passed"
    );

    let resumed = driver.resume(&id, &task, &Context::default()).unwrap();
    assert_eq!(resumed.session_id(), id);
    resumed.finish().unwrap();
    assert!(read("argv").contains(&format!("--resume\n{id}\n")));
}

#[test]
fn an_exit_without_a_result_is_an_error_carrying_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let program = stub(dir.path(), "echo 'Not logged in' >&2\nexit 1");
    let task = Task {
        cwd: dir.path().to_path_buf(),
        ..Task::default()
    };
    let err = ClaudeCode::with_program(program)
        .start(&task, &Context::default())
        .unwrap()
        .finish()
        .unwrap_err();
    match err {
        DriverError::Exited { stderr, status, .. } => {
            assert_eq!(stderr, "Not logged in");
            assert!(status.contains('1'), "{status}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_missing_binary_is_a_spawn_error() {
    let task = Task {
        cwd: std::env::temp_dir(),
        ..Task::default()
    };
    let err = ClaudeCode::with_program("/nonexistent/claude")
        .start(&task, &Context::default())
        .err()
        .unwrap();
    assert!(matches!(err, DriverError::Spawn { .. }), "{err:?}");
}

#[test]
fn stop_kills_a_running_turn() {
    let dir = tempfile::tempdir().unwrap();
    let program = stub(
        dir.path(),
        "echo '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s\",\"model\":\"m\",\"tools\":[]}'\nexec sleep 30",
    );
    let task = Task {
        cwd: dir.path().to_path_buf(),
        ..Task::default()
    };
    let mut turn = ClaudeCode::with_program(program)
        .start(&task, &Context::default())
        .unwrap();
    assert!(matches!(turn.next(), Some(Ok(TurnEvent::Started { .. }))));
    let started = std::time::Instant::now();
    turn.handle().stop().unwrap();
    assert!(matches!(turn.next(), Some(Err(DriverError::Exited { .. }))));
    assert!(turn.next().is_none());
    assert!(
        started.elapsed().as_secs() < 10,
        "did not wait for sleep 30"
    );
}

/// What a consumer of the trait does: start, and resume when the agent asked something.
fn ask_then_answer(driver: &mut dyn Driver, cwd: &Path) -> Vec<String> {
    let task = Task {
        prompt: "first".into(),
        cwd: cwd.to_path_buf(),
        ..Task::default()
    };
    let turn = driver.start(&task, &Context::default()).unwrap();
    let id = turn.session_id().to_string();
    let first = turn.finish().unwrap();
    let answer = Task {
        prompt: "the answer".into(),
        ..task
    };
    let second = driver
        .resume(&id, &answer, &Context::default())
        .unwrap()
        .finish()
        .unwrap();
    [first.text, second.text].into_iter().flatten().collect()
}

#[test]
fn the_replay_double_drives_a_consumer_and_logs_its_calls() {
    let mut driver = ReplayDriver::from_files([
        fixtures().join("text.jsonl"),
        fixtures().join("resume.jsonl"),
    ])
    .unwrap();
    let texts = ask_then_answer(&mut driver, Path::new("/w"));
    assert_eq!(texts, ["ready", "again"]);
    let calls = driver.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].resume, None);
    assert_eq!(
        calls[1].resume.as_deref(),
        Some("00000000-0000-4000-8000-000000000002")
    );
    assert_eq!(calls[1].task.prompt, "the answer");
    assert_eq!(driver.remaining(), 0);
}

#[test]
fn the_replay_double_plays_a_directory_in_name_order() {
    let mut driver = ReplayDriver::from_dir(fixtures()).unwrap();
    assert_eq!(driver.remaining(), 7);
    let task = Task::default();
    // denied.jsonl sorts first.
    let r = driver
        .start(&task, &Context::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(r.denials, ["Edit"]);
}

#[test]
fn a_truncated_recording_ends_without_a_result_and_an_empty_queue_errs() {
    let text = fixture("tools");
    let cut: String = text.lines().take(8).map(|l| format!("{l}\n")).collect();
    let mut driver = ReplayDriver::new([cut]);
    let err = driver
        .start(&Task::default(), &Context::default())
        .unwrap()
        .finish()
        .unwrap_err();
    assert!(matches!(err, DriverError::NoResult), "{err:?}");
    let err = driver
        .start(&Task::default(), &Context::default())
        .err()
        .unwrap();
    assert!(matches!(err, DriverError::Replay(_)), "{err:?}");
}

#[test]
fn the_pointer_names_the_driver_and_the_transcript() {
    let p = ClaudeCode::new().pointer(Path::new("/w"), "u-1");
    assert_eq!(p.driver, "claude-code");
    assert_eq!(p.session_id, "u-1");
    assert!(
        p.transcript_path
            .unwrap()
            .ends_with("projects/-w/u-1.jsonl")
    );
}

#[test]
fn interrupt_ends_a_running_turn() {
    let dir = tempfile::tempdir().unwrap();
    let program = stub(dir.path(), "exec sleep 30");
    let task = Task {
        cwd: dir.path().to_path_buf(),
        ..Task::default()
    };
    let turn = ClaudeCode::with_program(program)
        .start(&task, &Context::default())
        .unwrap();
    let handle = turn.handle();
    let started = std::time::Instant::now();
    let waiter = std::thread::spawn(move || turn.finish());
    std::thread::sleep(std::time::Duration::from_millis(200));
    handle.interrupt().unwrap();
    let err = waiter.join().unwrap().unwrap_err();
    assert!(matches!(err, DriverError::Exited { .. }), "{err:?}");
    assert!(
        started.elapsed().as_secs() < 10,
        "did not wait for sleep 30"
    );
}
