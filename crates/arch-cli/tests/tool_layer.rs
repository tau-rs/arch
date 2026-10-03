//! `arch hook` and `arch mcp` as Claude Code runs them: separate processes, JSON on stdin.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

const SESSION: &str = "s-1";
const ELEMENT: &str = "e0e1e2e3";

/// A git repo copied from smallsvc, with a plan whose element may write `src/app/pay.rs`.
fn worktree() -> (tempfile::TempDir, PathBuf) {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/arch-fixtures/repos/smallsvc");
    assert!(
        src.join("Cargo.toml").is_file(),
        "{} is missing: run scripts/fetch-fixtures.sh",
        src.display()
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dir(&src, &root);
    let plan = format!(
        r#"session = "{SESSION}"
intention = "charge once"
created = "2026-10-03T00:00:00Z"

[[elements]]
id = "{ELEMENT}"
label = "E1"
intention = "Charge the card once."
site = "src/app/pay.rs"
files = ["src/app/pay.rs"]
"#
    );
    let session = root.join(".arch/sessions").join(SESSION);
    std::fs::create_dir_all(&session).unwrap();
    std::fs::write(session.join("plan.toml"), plan).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.name", "Person"],
        &["config", "user.email", "person@example.com"],
        &["add", "-A"],
        &["commit", "-qm", "init"],
    ] {
        let ok = Command::new("git")
            .args(args)
            .current_dir(&root)
            .status()
            .unwrap();
        assert!(ok.success(), "git {args:?}");
    }
    (dir, root)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to.join(&name));
        } else {
            std::fs::copy(entry.path(), to.join(&name)).unwrap();
        }
    }
}

/// Run `arch <args> --worktree <root> --session s-1 --element <element>` with `stdin`.
fn arch(args: &[&str], root: &Path, element: &str, stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_arch"))
        .args(args)
        .args(["--worktree", root.to_str().unwrap(), "--session", SESSION])
        .args(["--element", element])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn write_call(root: &Path, file: &str) -> String {
    json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Write",
        "tool_input": { "file_path": root.join(file), "content": "x" },
    })
    .to_string()
}

#[test]
fn hook_pre_exits_two_with_the_reason_and_fails_closed() {
    let (_dir, root) = worktree();
    let out = arch(
        &["hook", "pre"],
        &root,
        ELEMENT,
        &write_call(&root, "src/app/new.rs"),
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("src/app/new.rs is outside element E1"),
        "{stderr}"
    );
    assert!(
        root.join(".arch/sessions/s-1/records/0001-denial.toml")
            .is_file()
    );

    let out = arch(
        &["hook", "pre"],
        &root,
        ELEMENT,
        &write_call(&root, "src/app/pay.rs"),
    );
    assert_eq!(out.status.code(), Some(2), "never read");

    let out = arch(
        &["hook", "pre"],
        &root,
        "ffffffff",
        &write_call(&root, "src/app/pay.rs"),
    );
    assert_eq!(out.status.code(), Some(2), "an unknown element blocks");
    assert!(String::from_utf8_lossy(&out.stderr).contains("arch hook: "));

    let out = arch(&["hook", "pre"], &root, ELEMENT, "not json");
    assert_eq!(out.status.code(), Some(2), "unreadable input blocks");

    let out = arch(
        &["hook", "post"],
        &root,
        "ffffffff",
        &write_call(&root, "src/app/pay.rs"),
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "a failed post hook blocks nothing"
    );
}

#[test]
fn mcp_round_trips_every_tool_over_stdio() {
    let (_dir, root) = worktree();
    let requests = [
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "read", "arguments": {"path": "src/app/pay.rs"}}}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "check", "arguments": {}}}),
        json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "ask", "arguments": {"questions": [{"text": "Retry?", "options": ["yes", "no"]}]}}}),
    ];
    let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
    std::fs::write(root.join("src/app/pay.rs"), "// charged once\n").unwrap();
    let out = arch(&["mcp"], &root, ELEMENT, &input);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let replies: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(replies.len(), 5);
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "arch");
    assert_eq!(replies[1]["result"]["tools"].as_array().unwrap().len(), 4);
    let text = |i: usize, j: usize| {
        replies[i]["result"]["content"][j]["text"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(
        text(2, 0).starts_with("src/app/pay.rs · sha256 "),
        "{}",
        text(2, 0)
    );
    assert_eq!(text(2, 1), "// charged once\n");
    let check: Value = serde_json::from_str(&text(3, 0)).unwrap();
    assert_eq!(check["schema_version"], 0, "the check.schema.json document");
    assert_eq!(text(4, 0), "wait");

    // The read counts: the pre hook now lets the element's write through.
    let out = arch(
        &["hook", "pre"],
        &root,
        ELEMENT,
        &write_call(&root, "src/app/pay.rs"),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let commit = json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "commit", "arguments": {"type": "fix", "summary": "charge the card once"}}});
    let out = arch(&["mcp"], &root, ELEMENT, &format!("{commit}\n"));
    let reply: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let said = reply["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        said.ends_with("fix(app): charge the card once"),
        "the area comes from areas.toml: {said}"
    );
}
