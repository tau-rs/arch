//! The acting replay (#48): recorded tool calls acted out through the context's hooks and MCP
//! server, with stub hooks and a stub server that log what they receive. `arch-cli`'s
//! `tests/session.rs` runs the same with the real `arch hook` and `arch mcp`.

use std::path::Path;

use arch_driver::{Context, Driver, ReplayDriver, Task, TurnEvent};
use serde_json::{Value, json};

fn line(v: Value) -> String {
    v.to_string()
}

/// One recorded turn: the given tool calls, then a success result.
fn turn(calls: &[(&str, Value)]) -> String {
    let mut out = vec![line(
        json!({ "type": "system", "subtype": "init", "session_id": "s-1",
        "model": "m", "tools": [] }),
    )];
    for (i, (name, input)) in calls.iter().enumerate() {
        out.push(line(json!({ "type": "assistant", "session_id": "s-1", "parent_tool_use_id": null,
            "message": { "role": "assistant", "content": [{ "type": "tool_use", "id": format!("t{i}"),
                "name": name, "input": input }] } })));
        out.push(line(json!({ "type": "user", "session_id": "s-1", "parent_tool_use_id": null,
            "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": format!("t{i}"),
                "content": "ok" }] } })));
    }
    out.push(line(
        json!({ "type": "result", "subtype": "success", "is_error": false,
        "session_id": "s-1", "result": "done", "num_turns": 1 }),
    ));
    out.join("\n")
}

fn script(dir: &Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    p.display().to_string()
}

#[test]
fn writes_pass_the_hooks_denials_write_nothing_and_mcp_calls_reach_the_server() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("work");
    std::fs::create_dir_all(work.join("src")).unwrap();
    std::fs::write(work.join("src/lib.rs"), "pub fn a() {}\n").unwrap();
    let log = tmp.path().join("log");
    let log_s = log.display().to_string();
    // The pre hook logs, and blocks any write to a path holding "secret".
    let pre = script(
        tmp.path(),
        "pre",
        &format!(
            "in=$(cat); printf 'pre %s\\n' \"$in\" >> '{log_s}'; case \"$in\" in *secret*) echo no >&2; exit 2;; esac"
        ),
    );
    let post = script(
        tmp.path(),
        "post",
        &format!("in=$(cat); printf 'post %s\\n' \"$in\" >> '{log_s}'"),
    );
    let server = script(
        tmp.path(),
        "server",
        &format!(
            "while read -r l; do printf 'mcp %s\\n' \"$l\" >> '{log_s}'; \
             echo '{{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{{}}}}'; done"
        ),
    );
    let settings = tmp.path().join("settings.json");
    std::fs::write(
        &settings,
        json!({ "hooks": {
            "PreToolUse": [{ "matcher": "Edit|Write", "hooks": [{ "type": "command", "command": pre }] }],
            "PostToolUse": [{ "matcher": "Read|Edit|Write", "hooks": [{ "type": "command", "command": post }] }],
        } })
        .to_string(),
    )
    .unwrap();
    let mcp = tmp.path().join("mcp.json");
    std::fs::write(
        &mcp,
        json!({ "mcpServers": { "arch": { "command": server, "args": [] } } }).to_string(),
    )
    .unwrap();

    let recorded = turn(&[
        ("Read", json!({ "file_path": "src/lib.rs" })),
        (
            "Write",
            json!({ "file_path": "src/refund.rs", "content": "pub struct Refund;\n" }),
        ),
        (
            "Edit",
            json!({ "file_path": "src/lib.rs", "old_string": "a()", "new_string": "b()" }),
        ),
        (
            "Write",
            json!({ "file_path": "src/secret.rs", "content": "nope\n" }),
        ),
        (
            "mcp__arch__commit",
            json!({ "type": "feat", "summary": "add Refund" }),
        ),
    ]);
    let mut driver = ReplayDriver::new([recorded]).acting();
    let task = Task {
        prompt: "go".into(),
        cwd: work.clone(),
        ..Task::default()
    };
    let context = Context {
        settings: Some(settings),
        mcp_config: Some(mcp),
        pack: None,
    };
    let events: Vec<TurnEvent> = driver
        .start(&task, &context)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(matches!(events.last(), Some(TurnEvent::Result(_))));

    assert_eq!(
        std::fs::read_to_string(work.join("src/refund.rs")).unwrap(),
        "pub struct Refund;\n"
    );
    assert_eq!(
        std::fs::read_to_string(work.join("src/lib.rs")).unwrap(),
        "pub fn b() {}\n"
    );
    assert!(
        !work.join("src/secret.rs").exists(),
        "a denied write is not applied"
    );

    let log = std::fs::read_to_string(&log).unwrap();
    let kinds: Vec<(&str, Value)> = log
        .lines()
        .map(|l| {
            let (kind, json) = l.split_once(' ').unwrap();
            (kind, serde_json::from_str(json).unwrap())
        })
        .collect();
    let summary: Vec<String> = kinds
        .iter()
        .map(|(k, v)| match *k {
            "mcp" => format!("mcp {}", v["method"].as_str().unwrap()),
            _ => format!(
                "{k} {} {}",
                v["tool_name"].as_str().unwrap(),
                Path::new(v["tool_input"]["file_path"].as_str().unwrap())
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
            ),
        })
        .collect();
    assert_eq!(
        summary,
        [
            "post Read lib.rs",
            "pre Write refund.rs",
            "post Write refund.rs",
            "pre Edit lib.rs",
            "post Edit lib.rs",
            "pre Write secret.rs",
            "mcp initialize",
            "mcp tools/call",
        ]
    );
    // Paths reach the hooks absolute, as Claude Code's do; the MCP call carries its arguments.
    let (_, write) = &kinds[1];
    assert_eq!(
        write["tool_input"]["file_path"],
        work.join("src/refund.rs").display().to_string()
    );
    assert_eq!(write["hook_event_name"], "PreToolUse");
    let (_, call) = kinds.last().unwrap();
    assert_eq!(call["params"]["name"], "commit");
    assert_eq!(call["params"]["arguments"]["summary"], "add Refund");
}

#[test]
fn without_acting_nothing_is_written() {
    let tmp = tempfile::tempdir().unwrap();
    let recorded = turn(&[("Write", json!({ "file_path": "a.rs", "content": "x" }))]);
    let mut driver = ReplayDriver::new([recorded]);
    let task = Task {
        cwd: tmp.path().to_path_buf(),
        ..Task::default()
    };
    driver
        .start(&task, &Context::default())
        .unwrap()
        .finish()
        .unwrap();
    assert!(!tmp.path().join("a.rs").exists());
}
