//! The tool layer (ADR 0012): the guard as a table, the hooks on a worktree, the commit tool on
//! a temp repo, and an MCP round trip for each tool.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

use arch_driver::tool_layer::commit::{CommitRequest, commit};
use arch_driver::tool_layer::config::{self, ToolLayerArgs};
use arch_driver::tool_layer::guard::{self, Disk, ToolCall, Verdict};
use arch_driver::tool_layer::hook::{self, Phase};
use arch_driver::tool_layer::mcp::{McpServer, TOOLS};
use arch_driver::tool_layer::{Project, Scope};
use arch_facts::{
    ArchDir, ContentHash, Element, ElementId, ElementState, Plan, RecordKind, SessionId,
    ThreadAuthor, ThreadEvent, ToolLayerState, tool_layer_states,
};
use serde_json::{Value, json};

fn element(files: &[&str]) -> Element {
    Element {
        id: ElementId::from_str_unchecked("e0e1e2e3"),
        label: "E1".into(),
        intention: "Charge the card once.".into(),
        site: "src/pay.rs".into(),
        files: files.iter().map(PathBuf::from).collect(),
        depends_on: vec![],
        state: ElementState::Running,
        resolve: false,
    }
}

// ---------------------------------------------------------------------------------------------
// The guard, as a table.

struct FakeDisk(HashMap<PathBuf, Vec<u8>>);

impl Disk for FakeDisk {
    fn locate(&self, raw: &Path) -> PathBuf {
        guard::normalize(&Path::new("/w").join(raw))
    }
    fn read(&self, rel: &Path) -> Option<Vec<u8>> {
        self.0.get(rel).cloned()
    }
}

fn call(tool: &str, input: Value) -> ToolCall {
    ToolCall {
        tool: tool.into(),
        input,
    }
}

enum Want {
    Allow(Option<&'static str>),
    Deny(&'static str),
}

#[test]
fn the_pre_hook_decides_as_the_table_says() {
    let scope = Scope {
        worktree: "/w".into(),
        session: SessionId::new("s-1"),
        element: element(&["src/pay.rs", "./src/new.rs"]),
    };
    let disk = FakeDisk(HashMap::from([
        ("src/pay.rs".into(), b"fn pay() {}\n".to_vec()),
        ("src/ship.rs".into(), b"fn ship() {}\n".to_vec()),
    ]));
    let mut state = ToolLayerState::new(scope.session.clone(), scope.element.id.clone());
    let read = |s: &mut ToolLayerState, p: &str, c: &str| {
        s.record_read(Path::new(p), ContentHash::of_str(c))
    };
    read(&mut state, "src/pay.rs", "fn pay() {}\n");
    read(&mut state, "src/ship.rs", "fn ship() {}\n");
    let mut stale = state.clone();
    read(&mut stale, "src/pay.rs", "fn pay() { old }\n");
    let unread = ToolLayerState::new(scope.session.clone(), scope.element.id.clone());

    let rows: Vec<(&str, &ToolLayerState, ToolCall, Want)> = vec![
        (
            "in scope + fresh → allow, expecting the edited content",
            &state,
            call(
                "Edit",
                json!({"file_path": "/w/src/pay.rs", "old_string": "{}", "new_string": "{ charge() }"}),
            ),
            Want::Allow(Some("fn pay() { charge() }\n")),
        ),
        (
            "out of scope → deny with the reason",
            &state,
            call(
                "Edit",
                json!({"file_path": "/w/src/ship.rs", "old_string": "{}", "new_string": "{ x }"}),
            ),
            Want::Deny(
                "src/ship.rs is outside element E1 (e0e1e2e3); it may write: src/pay.rs, ./src/new.rs",
            ),
        ),
        (
            "changed since read → deny re-read",
            &stale,
            call(
                "Write",
                json!({"file_path": "/w/src/pay.rs", "content": "x"}),
            ),
            Want::Deny("src/pay.rs changed since element E1 last read it; re-read it"),
        ),
        (
            "never read + exists → deny",
            &unread,
            call(
                "Write",
                json!({"file_path": "/w/src/pay.rs", "content": "x"}),
            ),
            Want::Deny("src/pay.rs exists and element E1 has not read it; read it first"),
        ),
        (
            "new file in scope → allow, expecting the written content",
            &unread,
            call(
                "Write",
                json!({"file_path": "src/new.rs", "content": "fn new() {}\n"}),
            ),
            Want::Allow(Some("fn new() {}\n")),
        ),
        (
            "new file out of scope → deny",
            &state,
            call(
                "Write",
                json!({"file_path": "/w/src/other.rs", "content": "x"}),
            ),
            Want::Deny("src/other.rs is outside element E1"),
        ),
        (
            "a path out of the worktree → deny",
            &state,
            call(
                "Write",
                json!({"file_path": "/w/../etc/passwd", "content": "x"}),
            ),
            Want::Deny("is outside the worktree /w"),
        ),
        (
            "git commit in Bash → deny",
            &state,
            call(
                "Bash",
                json!({"command": "git commit -am wip", "description": "save work"}),
            ),
            Want::Deny("git commit is denied to agents (ADR 0016)"),
        ),
        (
            "git push in Bash → deny",
            &state,
            call("Bash", json!({"command": "git push"})),
            Want::Deny("agents never push"),
        ),
        (
            "cargo test in Bash → allow",
            &state,
            call("Bash", json!({"command": "cargo test 2>&1 | tail -5"})),
            Want::Allow(None),
        ),
        (
            "Read is not guarded",
            &unread,
            call("Read", json!({"file_path": "/w/src/ship.rs"})),
            Want::Allow(None),
        ),
        (
            "an edit whose old text is absent → allow, nothing expected (the tool fails)",
            &state,
            call(
                "Edit",
                json!({"file_path": "/w/src/pay.rs", "old_string": "nope", "new_string": "x"}),
            ),
            Want::Allow(None),
        ),
    ];
    for (name, state, call, want) in rows {
        let got = guard::pre(&call, &scope, state, &disk);
        match (want, got) {
            (Want::Allow(content), Verdict::Allow { expect }) => assert_eq!(
                expect.map(|(_, h)| h),
                content.map(ContentHash::of_str),
                "{name}"
            ),
            (Want::Deny(says), Verdict::Deny(d)) => {
                assert!(d.reason.starts_with("arch: "), "{name}: {}", d.reason);
                assert!(d.reason.contains(says), "{name}: {}", d.reason);
            }
            (_, got) => panic!("{name}: got {got:?}"),
        }
    }
}

#[test]
fn the_post_hook_records_reads_confirms_writes_and_drops_failures() {
    let scope = Scope {
        worktree: "/w".into(),
        session: SessionId::new("s-1"),
        element: element(&["src/pay.rs"]),
    };
    let disk = FakeDisk(HashMap::from([("src/pay.rs".into(), b"new".to_vec())]));
    let mut state = ToolLayerState::new(scope.session.clone(), scope.element.id.clone());
    let pay = Path::new("src/pay.rs");

    guard::post(
        &call("Read", json!({"file_path": "/w/src/pay.rs"})),
        false,
        &scope,
        &mut state,
        &disk,
    );
    assert_eq!(state.last_read(pay), Some(&ContentHash::of_str("new")));

    state.expect(pay, ContentHash::of_str("never landed"));
    let write = call(
        "Write",
        json!({"file_path": "/w/src/pay.rs", "content": "new"}),
    );
    guard::post(&write, true, &scope, &mut state, &disk);
    assert!(
        state.expected.is_empty(),
        "a failed write is no longer expected"
    );
    assert!(state.writes.is_empty());

    guard::post(&write, false, &scope, &mut state, &disk);
    assert!(state.matches(pay, &ContentHash::of_str("new")));
    assert_eq!(state.writes.len(), 1, "the write is attributed");
}

// ---------------------------------------------------------------------------------------------
// The hooks on a real worktree.

struct Worktree {
    _dir: tempfile::TempDir,
    root: PathBuf,
    session: SessionId,
    element: ElementId,
}

/// A git repo with `src/pay.rs` and `src/ship.rs` committed and a plan whose element E1 may
/// write `src/pay.rs` and `src/new.rs`.
fn worktree() -> Worktree {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "Person"]);
    git(&root, &["config", "user.email", "person@example.com"]);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/pay.rs"), "fn pay() {}\n").unwrap();
    std::fs::write(root.join("src/ship.rs"), "fn ship() {}\n").unwrap();
    std::fs::write(root.join(".gitignore"), ".arch/cache/\n").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "init"]);

    let session = SessionId::new("s-1");
    let mut plan = Plan::new(session.clone(), "charge once");
    plan.elements.push(element(&["src/pay.rs", "src/new.rs"]));
    ArchDir::of_repo(&root)
        .session(&session)
        .write_plan(&plan)
        .unwrap();
    Worktree {
        _dir: dir,
        root,
        session,
        element: plan.elements[0].id.clone(),
    }
}

impl Worktree {
    fn scope(&self) -> Scope {
        Scope::load(&self.root, &self.session, &self.element).unwrap()
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn hook_input(event: &str, tool: &str, input: Value) -> String {
    json!({
        "session_id": "u-1",
        "cwd": "/somewhere",
        "hook_event_name": event,
        "tool_name": tool,
        "tool_input": input,
    })
    .to_string()
}

#[test]
fn a_denied_write_exits_two_with_the_reason_and_leaves_a_denial_record() {
    let wt = worktree();
    let scope = wt.scope();
    let ship = wt.root.join("src/ship.rs");
    let input = hook_input(
        "PreToolUse",
        "Write",
        json!({"file_path": ship, "content": "x"}),
    );
    let out = hook::run(Phase::Pre, &scope, &input).unwrap();
    assert_eq!(out.exit_code, 2);
    assert!(out.stderr.contains("has not read it"), "{}", out.stderr);

    let read = hook_input("PostToolUse", "Read", json!({"file_path": ship}));
    assert_eq!(hook::run(Phase::Post, &scope, &read).unwrap().exit_code, 0);
    let out = hook::run(Phase::Pre, &scope, &input).unwrap();
    assert_eq!(out.exit_code, 2);
    assert!(out.stderr.contains("outside element E1"), "{}", out.stderr);

    let records = ArchDir::of_repo(&wt.root)
        .session(&wt.session)
        .read_records()
        .unwrap();
    assert_eq!(records.len(), 2);
    let RecordKind::Denial {
        element,
        path,
        reason,
        ..
    } = &records[1].kind
    else {
        panic!("{:?}", records[1]);
    };
    assert_eq!(element, &wt.element);
    assert_eq!(path, Path::new("src/ship.rs"));
    assert!(reason.contains("outside element E1"));
}

#[test]
fn an_allowed_write_is_expected_before_it_lands_and_confirmed_after() {
    let wt = worktree();
    let scope = wt.scope();
    let new = wt.root.join("src/new.rs");
    let input = json!({"file_path": new, "content": "fn new() {}\n"});
    let out = hook::run(
        Phase::Pre,
        &scope,
        &hook_input("PreToolUse", "Write", input.clone()),
    )
    .unwrap();
    assert_eq!(
        out,
        hook::HookOutcome {
            exit_code: 0,
            stderr: String::new()
        }
    );

    let hash = ContentHash::of_str("fn new() {}\n");
    let states = tool_layer_states(&wt.root);
    assert_eq!(states.len(), 1);
    assert!(
        states[0].matches(Path::new("src/new.rs"), &hash),
        "the watcher sees it before the write"
    );
    assert_eq!(states[0].session, wt.session);

    std::fs::write(&new, "fn new() {}\n").unwrap();
    hook::run(
        Phase::Post,
        &scope,
        &hook_input("PostToolUse", "Write", input),
    )
    .unwrap();
    let state = &tool_layer_states(&wt.root)[0];
    assert_eq!(state.writes.len(), 1);
    assert_eq!(
        state.last_read(Path::new("src/new.rs")),
        Some(&hash),
        "its own write counts as read"
    );
}

#[test]
fn an_unknown_element_is_an_error_the_cli_turns_into_a_block() {
    let wt = worktree();
    let err = Scope::load(
        &wt.root,
        &wt.session,
        &ElementId::from_str_unchecked("ffffffff"),
    )
    .unwrap_err();
    assert!(err.to_string().contains("no element ffffffff"), "{err}");
}

// ---------------------------------------------------------------------------------------------
// The commit tool.

fn log(dir: &Path) -> Vec<String> {
    git(dir, &["log", "--format=%s"])
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn commit_stages_only_the_elements_files_with_the_trailers_and_amends_its_own_head() {
    let wt = worktree();
    let scope = wt.scope();
    std::fs::write(wt.root.join("src/pay.rs"), "fn pay() { charge() }\n").unwrap();
    std::fs::write(wt.root.join("src/new.rs"), "fn new() {}\n").unwrap();
    std::fs::write(wt.root.join("src/ship.rs"), "fn ship() { changed }\n").unwrap();
    std::fs::write(wt.root.join("staged.rs"), "staged by someone\n").unwrap();
    git(&wt.root, &["add", "staged.rs"]);

    let request = CommitRequest {
        kind: "feat".into(),
        summary: "charge the card once".into(),
        body: None,
    };
    let done = commit(
        &scope,
        &request,
        Some("billing"),
        "Claude <noreply@anthropic.com>",
    )
    .unwrap();
    assert!(!done.amended);
    assert_eq!(
        done.files,
        [PathBuf::from("src/new.rs"), PathBuf::from("src/pay.rs")]
    );
    let message = git(&wt.root, &["log", "-1", "--format=%B"]);
    assert_eq!(
        message.trim_end(),
        "feat(billing): charge the card once\n\nCharge the card once.\n\n\
         Arch-Element: e0e1e2e3\nArch-Session: s-1\nCo-authored-by: Claude <noreply@anthropic.com>"
    );
    let status = git(&wt.root, &["status", "--porcelain"]);
    assert!(status.contains(" M src/ship.rs"), "{status}");
    assert!(
        status.contains("A  staged.rs"),
        "someone else's staged file stays staged: {status}"
    );

    std::fs::write(wt.root.join("src/pay.rs"), "fn pay() { charge_once() }\n").unwrap();
    let again = CommitRequest {
        kind: "fix".into(),
        summary: "charge once".into(),
        body: Some("Retry without a second debit.".into()),
    };
    let done = commit(&scope, &again, None, "Claude <noreply@anthropic.com>").unwrap();
    assert!(done.amended);
    assert_eq!(
        log(&wt.root),
        ["fix: charge once", "init"],
        "one commit per element"
    );
    assert_eq!(
        done.files.len(),
        2,
        "the amend keeps the first call's files"
    );
    assert!(git(&wt.root, &["log", "-1", "--format=%B"]).contains("Retry without a second debit."));

    git(&wt.root, &["commit", "-qm", "chore: by hand"]);
    std::fs::write(wt.root.join("src/pay.rs"), "fn pay() { v3 }\n").unwrap();
    let done = commit(&scope, &request, None, "Claude <noreply@anthropic.com>").unwrap();
    assert!(!done.amended, "HEAD is not the element's commit any more");
    assert_eq!(log(&wt.root).len(), 4);
}

#[test]
fn commit_refuses_a_bad_type_and_an_empty_change() {
    let wt = worktree();
    let scope = wt.scope();
    let bad = CommitRequest {
        kind: "wip".into(),
        summary: "x".into(),
        body: None,
    };
    assert!(
        commit(&scope, &bad, None, "c")
            .unwrap_err()
            .to_string()
            .contains("not one of")
    );
    let nothing = CommitRequest {
        kind: "feat".into(),
        ..bad
    };
    assert!(
        commit(&scope, &nothing, None, "c")
            .unwrap_err()
            .to_string()
            .contains("nothing to commit")
    );
}

// ---------------------------------------------------------------------------------------------
// MCP over stdio.

struct FakeProject;

impl Project for FakeProject {
    fn check(&self, _: &Path) -> Result<Value, String> {
        Ok(
            json!({"schema_version": 0, "summary": {"blocking": 0, "warnings": 1, "allowed": 0}, "findings": []}),
        )
    }
    fn area_of(&self, _: &Path, file: &Path) -> Option<String> {
        file.starts_with("src").then(|| "billing".into())
    }
}

/// Send `requests` as lines through `serve`; the replies, one per request with an id.
fn exchange(server: &McpServer, requests: &[Value]) -> Vec<Value> {
    let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
    let mut output = vec![];
    server.serve(Cursor::new(input), &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn tool(id: u32, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
}

fn texts(reply: &Value) -> Vec<String> {
    reply["result"]["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["text"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn mcp_answers_initialize_list_and_each_tool() {
    let wt = worktree();
    let server = McpServer::new(wt.scope(), &FakeProject, "Claude <noreply@anthropic.com>");
    let replies = exchange(
        &server,
        &[
            json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
            tool(2, "read", json!({"path": "src/pay.rs"})),
            tool(3, "check", json!({})),
            tool(
                4,
                "ask",
                json!({"questions": [{"text": "Retry or refund?", "options": ["retry: one more debit", "refund: none"]}]}),
            ),
            json!({"jsonrpc": "2.0", "id": 5, "method": "nope"}),
            tool(6, "rm", json!({})),
            tool(7, "read", json!({"path": "../outside"})),
        ],
    );
    assert_eq!(replies.len(), 8, "the notification gets no answer");
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "arch");

    let names: Vec<&str> = replies[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["read", "check", "commit", "ask"]);
    assert_eq!(
        TOOLS,
        names
            .iter()
            .map(|n| format!("mcp__arch__{n}"))
            .collect::<Vec<_>>()
    );

    let read = texts(&replies[2]);
    let hash = ContentHash::of_str("fn pay() {}\n");
    assert_eq!(
        read,
        [
            format!("src/pay.rs · sha256 {hash}"),
            "fn pay() {}\n".into()
        ]
    );
    let state = &tool_layer_states(&wt.root)[0];
    assert_eq!(
        state.last_read(Path::new("src/pay.rs")),
        Some(&hash),
        "read records the hash"
    );

    let check: Value = serde_json::from_str(&texts(&replies[3])[0]).unwrap();
    assert_eq!(check["summary"]["warnings"], 1);

    assert_eq!(texts(&replies[4]), ["wait"]);
    let thread = ArchDir::of_repo(&wt.root)
        .session(&wt.session)
        .read_thread()
        .unwrap();
    assert_eq!(thread.len(), 1);
    assert_eq!(
        thread[0].author,
        ThreadAuthor::Agent {
            element: Some(wt.element.clone())
        }
    );
    let ThreadEvent::Ask { questions } = &thread[0].event else {
        panic!("{:?}", thread[0]);
    };
    assert_eq!(questions[0].options.len(), 2);

    assert_eq!(replies[5]["error"]["code"], -32601);
    assert_eq!(replies[6]["error"]["code"], -32602);
    assert_eq!(replies[7]["result"]["isError"], true);
    assert!(texts(&replies[7])[0].contains("outside the worktree"));
}

#[test]
fn mcp_commit_uses_the_area_from_the_port_and_reports_errors_to_the_agent() {
    let wt = worktree();
    let server = McpServer::new(wt.scope(), &FakeProject, "Claude <noreply@anthropic.com>");
    std::fs::write(wt.root.join("src/pay.rs"), "fn pay() { charge() }\n").unwrap();
    let replies = exchange(
        &server,
        &[
            tool(
                1,
                "commit",
                json!({"type": "feat", "summary": "charge the card once"}),
            ),
            tool(2, "commit", json!({"type": "feat"})),
        ],
    );
    let said = &texts(&replies[0])[0];
    assert!(said.starts_with("committed "), "{said}");
    assert!(
        said.ends_with("feat(billing): charge the card once"),
        "{said}"
    );
    assert_eq!(replies[1]["result"]["isError"], true);
    assert!(texts(&replies[1])[0].contains("`summary` is required"));
}

// ---------------------------------------------------------------------------------------------
// The files handed to the driver.

#[test]
fn settings_and_mcp_config_point_at_the_binary_on_this_element() {
    let wt = worktree();
    let args = ToolLayerArgs {
        exe: "/opt/arch it's/arch".into(),
        worktree: wt.root.clone(),
        session: wt.session.clone(),
        element: wt.element.clone(),
    };
    let context = config::write_context(&args, None).unwrap();
    let settings: Value =
        serde_json::from_str(&std::fs::read_to_string(context.settings.unwrap()).unwrap()).unwrap();
    let pre = settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert_eq!(
        pre,
        format!(
            r"'/opt/arch it'\''s/arch' 'hook' 'pre' '--worktree' '{}' '--session' 's-1' '--element' 'e0e1e2e3'",
            wt.root.display()
        )
    );
    assert_eq!(
        settings["hooks"]["PreToolUse"][0]["matcher"],
        "Edit|Write|MultiEdit|NotebookEdit|Bash"
    );
    assert!(
        settings["hooks"]["PostToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("'hook' 'post'")
    );
    assert!(settings["hooks"]["PostToolUseFailure"].is_array());
    assert_eq!(
        settings["permissions"]["deny"],
        json!(["Bash(git commit:*)", "Bash(git push:*)"])
    );

    let mcp: Value =
        serde_json::from_str(&std::fs::read_to_string(context.mcp_config.unwrap()).unwrap())
            .unwrap();
    let server = &mcp["mcpServers"]["arch"];
    assert_eq!(server["command"], "/opt/arch it's/arch");
    assert_eq!(server["args"][0], "mcp");
    assert_eq!(server["args"][3], "--session");
    assert_eq!(server["args"][7], "--co-author");

    let cached = wt.root.join(".arch/cache/driver/e0e1e2e3");
    assert!(cached.join("settings.json").is_file() && cached.join("mcp.json").is_file());
    assert!(
        tool_layer_states(&wt.root).is_empty(),
        "the driver files are not tool-layer state"
    );
}
