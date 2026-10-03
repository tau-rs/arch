//! Opt-in smoke test against the real `claude` CLI: needs a Claude Code login, costs a few cents,
//! never runs in default CI.
//!
//! ```sh
//! ARCH_SMOKE_CLAUDE=1 cargo test -p arch-driver --test smoke -- --ignored
//! ```

use arch_driver::{ClaudeCode, Context, Driver, Task, TurnEvent};

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
