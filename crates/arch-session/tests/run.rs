//! The scheduler, the gate, the judge and fix rounds with the replay double (issue #47, "Done
//! when"): recorded stream-json from `crates/arch-driver/tests/fixtures/`, one recording per
//! element turn, the judge's answer swapped into the `structured` recording.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::result::Result;

use arch_driver::{Context, Driver, DriverError, ReplayCall, ReplayDriver, Task, Turn};
use arch_facts::*;
use arch_session::{AcceptOptions, Door, Engine, EngineOptions, Error, Project, Typology, accept};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

// ---------------------------------------------------------------- recorded turns

fn fixture(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../arch-driver/tests/fixtures")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

const RECORDED_ID: &str = "00000000-0000-4000-8000-000000000002";

/// The recorded `text` turn, as driver session `sid`.
fn element_turn(sid: &str) -> String {
    fixture("text.jsonl").replace(RECORDED_ID, sid)
}

/// The `text` turn with a call to the `ask` tool before its result.
fn ask_turn(sid: &str) -> String {
    let call = json!({ "type": "assistant", "session_id": sid, "parent_tool_use_id": null,
        "message": { "role": "assistant", "content": [{ "type": "tool_use", "id": "toolu_ask",
            "name": "mcp__arch__ask",
            "input": { "questions": [{ "text": "Refunds in cents or in units?",
                                       "options": ["cents", "units"] }] } }] } });
    let mut out = String::new();
    for line in element_turn(sid).lines() {
        if line.contains("\"type\":\"result\"") || line.contains("\"type\": \"result\"") {
            out.push_str(&call.to_string());
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The recorded `structured` turn, answering `verdicts` as `(element, pass?)`.
fn judge_turn(sid: &str, verdicts: &[(&ElementId, bool)]) -> String {
    let answer = json!({ "verdicts": verdicts.iter().map(|(e, pass)| json!({
        "element": e.as_str(),
        "verdict": if *pass { "pass" } else { "fail" },
        "reason": if *pass { "realized as intended" } else { "the refund is never persisted" },
    })).collect::<Vec<_>>() });
    fixture("structured.jsonl")
        .replace(RECORDED_ID, sid)
        .lines()
        .map(|l| {
            let mut v: Value = serde_json::from_str(l).unwrap();
            if v["type"] == "result" {
                v["structured_output"] = answer.clone();
            }
            v.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------- the test's world

/// The port with no analyzer: a clean `arch check`, and facts only when given.
struct FakeProject {
    blocking: u64,
    facts: Option<Facts>,
}

impl Project for FakeProject {
    fn check(&self, _: &Path) -> Result<Value, String> {
        Ok(
            json!({ "schema_version": 0, "summary": { "blocking": self.blocking, "warnings": 0, "allowed": 0 }, "findings": [] }),
        )
    }
    fn facts(&self, _: &Path) -> Result<Facts, String> {
        self.facts
            .clone()
            .ok_or_else(|| "no analyzer in this test".into())
    }
}

const CLEAN: FakeProject = FakeProject {
    blocking: 0,
    facts: None,
};

/// The replay double, plus what arch's hooks would write while a turn runs: before call `n`,
/// the given Denial records.
struct Hooked {
    replay: ReplayDriver,
    dir: SessionDir,
    denials: Vec<(usize, RecordKind)>,
    calls: usize,
}

impl Hooked {
    fn before(&mut self) {
        for (n, kind) in &self.denials {
            if *n == self.calls {
                self.dir.write_record(kind.clone(), vec![]).unwrap();
            }
        }
        self.calls += 1;
    }
}

impl Driver for Hooked {
    fn name(&self) -> &'static str {
        "replay"
    }
    fn start(&mut self, task: &Task, context: &Context) -> Result<Turn, DriverError> {
        self.before();
        self.replay.start(task, context)
    }
    fn resume(&mut self, sid: &str, task: &Task, context: &Context) -> Result<Turn, DriverError> {
        self.before();
        self.replay.resume(sid, task, context)
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct World {
    _tmp: tempfile::TempDir,
    worktree: PathBuf,
    session: SessionId,
    ids: Vec<ElementId>,
}

/// An accepted session on a temp repo: `n` elements (E2 depends on E1 when `chain`), the gate's
/// command `test_command`.
fn world(n: usize, chain: bool, test_command: &str) -> World {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("smallsvc");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn a() {}\n").unwrap();
    std::fs::write(repo.join(".gitignore"), ".arch/cache/\n").unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let store = Store::open(&repo.join(".arch")).unwrap();
    let session = SessionId::new("3c9e1f0a");
    let mut plan = Plan::new(session.clone(), "add a refund flow");
    for i in 1..=n {
        let id = plan
            .add_element(format!("step {i}"), format!("src/e{i}.rs"))
            .id
            .clone();
        plan.elements[i - 1].files = vec![format!("src/e{i}.rs").into()];
        let _ = id;
    }
    if chain {
        let e1 = plan.elements[0].id.clone();
        plan.elements[1].depends_on.push(e1);
    }
    store.plan_draft_put(&plan).unwrap();
    let s = accept(
        &repo,
        &store,
        &session,
        &[],
        &AcceptOptions {
            test_command: test_command.into(),
            ..AcceptOptions::default()
        },
    )
    .unwrap();
    World {
        _tmp: tmp,
        worktree: s.worktree.unwrap(),
        session,
        ids: plan.elements.iter().map(|e| e.id.clone()).collect(),
    }
}

impl World {
    fn dir(&self) -> SessionDir {
        ArchDir::of_repo(&self.worktree).session(&self.session)
    }

    fn driver(&self, turns: Vec<String>) -> Hooked {
        Hooked {
            replay: ReplayDriver::new(turns),
            dir: self.dir(),
            denials: vec![],
            calls: 0,
        }
    }

    fn engine<'a>(&self, driver: &'a mut Hooked, project: &'a FakeProject) -> Engine<'a> {
        Engine::open(
            &self.worktree,
            &self.session,
            driver,
            project,
            EngineOptions {
                exe: Some("/usr/local/bin/arch".into()),
                ..EngineOptions::default()
            },
        )
        .unwrap()
    }

    fn element_turns(&self) -> Vec<String> {
        (1..=self.ids.len())
            .map(|i| element_turn(&format!("agent-{i}")))
            .collect()
    }

    fn verdicts(&self) -> Vec<(ElementId, Verdict)> {
        self.dir()
            .read_records()
            .unwrap()
            .into_iter()
            .filter_map(|r| match r.kind {
                RecordKind::JudgeVerdict {
                    element, verdict, ..
                } => Some((element, verdict)),
                _ => None,
            })
            .collect()
    }
}

fn pass_all(ids: &[ElementId]) -> Vec<(&ElementId, bool)> {
    ids.iter().map(|e| (e, true)).collect()
}

fn resumed(calls: &[ReplayCall]) -> Vec<Option<String>> {
    calls.iter().map(|c| c.resume.clone()).collect()
}

// ---------------------------------------------------------------- the scenarios

#[test]
fn four_elements_one_gate_a_pass_verdict_is_done() {
    let w = world(4, false, "true");
    let mut turns = w.element_turns();
    turns.push(judge_turn("judge-1", &pass_all(&w.ids)));
    let mut driver = w.driver(turns);
    let mut engine = w.engine(&mut driver, &CLEAN);

    assert_eq!(engine.run().unwrap(), SessionState::Done);
    let session = engine.session().clone();
    let plan = engine.plan().clone();
    drop(engine);

    // One driver session per element, each started once, then the judge.
    let calls = driver.replay.calls();
    assert_eq!(calls.len(), 5);
    assert_eq!(resumed(calls), vec![None; 5]);
    for (i, id) in w.ids.iter().enumerate() {
        assert_eq!(
            session.agent(id).unwrap().session_id,
            format!("agent-{}", i + 1)
        );
        let call = &calls[i];
        assert!(call.task.prompt.contains(&format!("Element E{}", i + 1)));
        assert_eq!(
            call.task.tools,
            ["Read", "Edit", "Write", "Glob", "Grep", "Bash"]
        );
        assert!(
            call.task
                .allowed_tools
                .contains(&"mcp__arch__commit".to_string())
        );
        let settings = call.context.settings.as_ref().unwrap();
        assert!(settings.ends_with(format!(".arch/cache/driver/{id}/settings.json")));
        let hooks = std::fs::read_to_string(settings).unwrap();
        assert!(
            hooks.contains(&format!("'--element' '{id}'")),
            "hooks on the element"
        );
        assert!(call.context.mcp_config.as_ref().unwrap().is_file());
    }
    let judge = &calls[4];
    assert_eq!(
        judge.task.tools,
        ["Read", "Glob", "Grep"],
        "the judge only reads"
    );
    assert!(judge.task.output_schema.is_some());
    assert_eq!(judge.context.settings, None);
    assert!(
        judge
            .task
            .prompt
            .contains("You do not write, fix or propose code")
    );

    assert!(plan.elements.iter().all(|e| e.state == ElementState::Done));
    let records = w.dir().read_records().unwrap();
    let gate: Vec<_> = records
        .iter()
        .filter_map(|r| match &r.kind {
            RecordKind::GateOutput {
                command, exit_code, ..
            } => Some((command.as_str(), *exit_code)),
            _ => None,
        })
        .collect();
    assert_eq!(gate, [("true", 0), ("arch check", 0)]);
    assert!(matches!(&records[0].witnesses[..], [Witness::Tool { tool, .. }] if tool == "true"));
    assert_eq!(
        w.verdicts(),
        w.ids
            .iter()
            .map(|e| (e.clone(), Verdict::Pass))
            .collect::<Vec<_>>()
    );

    // The thread: arch's lines, the agents' filtered streams, the judge.
    let thread = w.dir().read_thread().unwrap();
    assert!(thread.iter().any(|t| matches!(&t.event,
        ThreadEvent::Text { text } if text == "gate · group 1 · true ✓ · arch check ✓ · judge 4/4 pass")));
    assert_eq!(
        thread
            .iter()
            .filter(|t| t.author == ThreadAuthor::Judge)
            .count(),
        4
    );
    let agent_lines = thread
        .iter()
        .filter(|t| matches!(&t.author, ThreadAuthor::Agent { element: Some(e) } if e == &w.ids[0]))
        .collect::<Vec<_>>();
    assert!(!agent_lines.is_empty());
    assert!(
        agent_lines
            .iter()
            .all(|t| t.driver.as_ref().unwrap().session_id == "agent-1")
    );

    // The folder is committed when the run stops.
    assert_eq!(
        git(&w.worktree, &["log", "-1", "--format=%s"]),
        "chore(arch): add a refund flow · done"
    );
    assert_eq!(git(&w.worktree, &["status", "--porcelain"]), "");
}

#[test]
fn a_fail_verdict_sends_the_element_back_for_one_fix_round_then_passes() {
    let w = world(4, false, "true");
    let mut turns = w.element_turns();
    let mut first = pass_all(&w.ids);
    first[1].1 = false;
    turns.push(judge_turn("judge-1", &first));
    turns.push(element_turn("agent-2"));
    turns.push(judge_turn("judge-2", &pass_all(&w.ids)));
    let mut driver = w.driver(turns);
    let mut engine = w.engine(&mut driver, &CLEAN);

    assert_eq!(engine.run().unwrap(), SessionState::Done);
    assert_eq!(engine.session().cursor.fix_round, 1);
    drop(engine);
    let calls = driver.replay.calls();
    assert_eq!(calls.len(), 7);
    let fix = &calls[5];
    assert_eq!(
        fix.resume.as_deref(),
        Some("agent-2"),
        "E2's own driver session is resumed"
    );
    assert!(fix.task.prompt.contains("fix round 1"));
    assert!(
        fix.task
            .prompt
            .contains("The judge on E2: the refund is never persisted")
    );
    assert_eq!(w.verdicts().len(), 8, "two gates, four verdicts each");
}

#[test]
fn two_fails_spend_the_budget_and_ask_the_four_doors() {
    let w = world(4, false, "true");
    let fail_e2 = |sid: &str| {
        let mut v = pass_all(&w.ids);
        v[1].1 = false;
        judge_turn(sid, &v)
    };
    let mut turns = w.element_turns();
    turns.push(fail_e2("judge-1"));
    turns.push(element_turn("agent-2"));
    turns.push(fail_e2("judge-2"));
    turns.push(element_turn("agent-2"));
    turns.push(fail_e2("judge-3"));
    let mut driver = w.driver(turns);
    let mut engine = w.engine(&mut driver, &CLEAN);

    assert_eq!(engine.run().unwrap(), SessionState::GateFailed);
    assert_eq!(engine.session().cursor.fix_round, 2);
    assert_eq!(engine.session().cursor.todo, [w.ids[1].clone()]);
    drop(engine);
    assert_eq!(driver.replay.remaining(), 0);
    let last = w.dir().read_thread().unwrap().pop().unwrap();
    assert_eq!(last.author, ThreadAuthor::Arch);
    let ThreadEvent::Ask { questions } = last.event else {
        panic!("the four-door question, got {last:?}");
    };
    assert_eq!(
        questions[0].options,
        [
            "one more round, with a hint",
            "take over",
            "accept as is, with a recorded reason",
            "re-plan"
        ]
    );
    assert_eq!(
        git(&w.worktree, &["log", "-1", "--format=%s"]),
        "chore(arch): add a refund flow · gate-failed"
    );

    // Door 1: one more round with a hint, then the gate passes.
    let mut driver = w.driver(vec![
        element_turn("agent-2"),
        judge_turn("judge-4", &pass_all(&w.ids)),
    ]);
    let mut engine = w.engine(&mut driver, &CLEAN);
    let state = engine
        .decide_gate(Door::OneMore {
            hint: Some("persist it in refunds".into()),
        })
        .unwrap();
    assert_eq!(state, SessionState::Done);
    drop(engine);
    let fix = &driver.replay.calls()[0];
    assert_eq!(fix.resume.as_deref(), Some("agent-2"));
    assert!(
        fix.task
            .prompt
            .contains("The person's hint: persist it in refunds")
    );
}

#[test]
fn accept_as_is_writes_an_override_and_finishes() {
    let w = world(1, false, "false");
    let fail = || judge_turn("judge", &[(&w.ids[0], false)]);
    let mut driver = w.driver(vec![
        element_turn("agent-1"),
        fail(),
        element_turn("agent-1"),
        fail(),
        element_turn("agent-1"),
        fail(),
    ]);
    let mut engine = w.engine(&mut driver, &CLEAN);
    assert_eq!(engine.run().unwrap(), SessionState::GateFailed);

    assert!(matches!(
        engine.decide_gate(Door::TakeOver),
        Err(Error::NotYet { issue: "#49", .. })
    ));
    assert!(matches!(
        engine.decide_gate(Door::Replan),
        Err(Error::NotYet { issue: "#49", .. })
    ));
    let state = engine
        .decide_gate(Door::AcceptAsIs {
            reason: "the flaky test is tracked in #12".into(),
            by: "titouan".into(),
        })
        .unwrap();
    assert_eq!(state, SessionState::Done);
    let r = w.dir().read_records().unwrap().pop().unwrap();
    assert_eq!(
        r.kind,
        RecordKind::Override {
            what: "gate of group 1".into(),
            reason: "the flaky test is tracked in #12".into(),
            by: "titouan".into(),
        }
    );
}

#[test]
fn a_failing_command_with_no_failing_verdict_sends_the_whole_group_back() {
    let w = world(2, false, "echo boom; exit 3");
    let mut turns = w.element_turns();
    turns.push(judge_turn("judge-1", &pass_all(&w.ids)));
    turns.push(element_turn("agent-1"));
    turns.push(element_turn("agent-2"));
    turns.push(judge_turn("judge-2", &pass_all(&w.ids)));
    turns.push(element_turn("agent-1"));
    turns.push(element_turn("agent-2"));
    turns.push(judge_turn("judge-3", &pass_all(&w.ids)));
    let mut driver = w.driver(turns);
    let mut engine = w.engine(&mut driver, &CLEAN);
    assert_eq!(engine.run().unwrap(), SessionState::GateFailed);
    drop(engine);
    let calls = driver.replay.calls();
    assert_eq!(
        resumed(&calls[3..5]),
        [Some("agent-1".into()), Some("agent-2".into())]
    );
    assert!(calls[3].task.prompt.contains("echo boom; exit 3 · exit 3"));
    assert!(calls[3].task.prompt.contains("boom"));
}

#[test]
fn arch_check_blocking_fails_the_gate() {
    let w = world(1, false, "");
    let blocking = FakeProject {
        blocking: 2,
        facts: None,
    };
    let fail_free = || judge_turn("judge", &[(&w.ids[0], true)]);
    let mut driver = w.driver(vec![
        element_turn("agent-1"),
        fail_free(),
        element_turn("agent-1"),
        fail_free(),
        element_turn("agent-1"),
        fail_free(),
    ]);
    let mut engine = w.engine(&mut driver, &blocking);
    assert_eq!(engine.run().unwrap(), SessionState::GateFailed);
    let gate = w.dir().read_records().unwrap();
    assert!(gate.iter().any(|r| matches!(&r.kind,
        RecordKind::GateOutput { command, exit_code: 1, .. } if command == "arch check")));
}

#[test]
fn groups_run_one_after_another_each_with_its_gate() {
    let w = world(2, true, "true");
    let mut driver = w.driver(vec![
        element_turn("agent-1"),
        judge_turn("judge-1", &[(&w.ids[0], true)]),
        element_turn("agent-2"),
        judge_turn("judge-2", &[(&w.ids[1], true)]),
    ]);
    let mut engine = w.engine(&mut driver, &CLEAN);
    assert_eq!(engine.run().unwrap(), SessionState::Done);
    assert_eq!(engine.session().cursor.group, 1);
    drop(engine);
    let judged: Vec<_> = driver
        .replay
        .calls()
        .iter()
        .filter(|c| c.task.output_schema.is_some())
        .map(|c| c.task.prompt.contains("E2 · id"))
        .collect();
    assert_eq!(judged, [false, true], "each gate judges its own group");
}

#[test]
fn an_ask_waits_for_the_answer_then_resumes_that_element() {
    let w = world(2, false, "true");
    let mut driver = w.driver(vec![ask_turn("agent-1")]);
    let mut engine = w.engine(&mut driver, &CLEAN);
    assert_eq!(engine.run().unwrap(), SessionState::Asks);
    assert_eq!(
        engine.session().cursor.waiting.as_ref().unwrap().element,
        w.ids[0]
    );
    assert!(matches!(
        engine.decide_deviation(Typology::BackOnPlan),
        Err(Error::IllegalTransition { .. })
    ));
    drop(engine);
    assert_eq!(
        git(&w.worktree, &["log", "-1", "--format=%s"]),
        "chore(arch): add a refund flow · asks"
    );

    // Another process answers.
    let mut driver = w.driver(vec![
        element_turn("agent-1"),
        element_turn("agent-2"),
        judge_turn("judge", &pass_all(&w.ids)),
    ]);
    let mut engine = w.engine(&mut driver, &CLEAN);
    let state = engine.answer(vec!["cents".into()]).unwrap();
    assert_eq!(state, SessionState::Done);
    drop(engine);
    let resume = &driver.replay.calls()[0];
    assert_eq!(resume.resume.as_deref(), Some("agent-1"));
    assert!(resume.task.prompt.contains("1. cents"));
    let thread = w.dir().read_thread().unwrap();
    assert!(thread.iter().any(|t| t.author == ThreadAuthor::You
        && t.event
            == ThreadEvent::Answer {
                answers: vec!["cents".into()]
            }));
}

#[test]
fn a_scope_denial_is_a_deviation_a_stale_one_is_not() {
    let w = world(2, false, "true");
    let mut driver = w.driver(vec![element_turn("agent-1"), element_turn("agent-2")]);
    driver.denials = vec![
        (
            0,
            RecordKind::Denial {
                element: w.ids[0].clone(),
                path: "src/lib.rs".into(),
                reason:
                    "arch: src/lib.rs changed since element E1 last read it; re-read it, then retry"
                        .into(),
                agent_reason: None,
            },
        ),
        (
            1,
            RecordKind::Denial {
                element: w.ids[1].clone(),
                path: "src/lib.rs".into(),
                reason: format!(
                    "arch: src/lib.rs is outside element E2 ({}); it may write: src/e2.rs",
                    w.ids[1]
                ),
                agent_reason: None,
            },
        ),
    ];
    let mut engine = w.engine(&mut driver, &CLEAN);
    assert_eq!(engine.run().unwrap(), SessionState::Deviation);
    let waiting = engine.session().cursor.waiting.clone().unwrap();
    assert_eq!(
        waiting.element, w.ids[1],
        "E1's stale write is not a deviation"
    );
    assert_eq!(waiting.denied, [PathBuf::from("src/lib.rs")]);
    drop(engine);

    let mut driver = w.driver(vec![
        element_turn("agent-2"),
        judge_turn("judge", &pass_all(&w.ids)),
    ]);
    let mut engine = w.engine(&mut driver, &CLEAN);
    assert_eq!(
        engine.decide_deviation(Typology::UpdatePlan).unwrap(),
        SessionState::Done
    );
    assert_eq!(
        engine.plan().element(&w.ids[1]).unwrap().files,
        [PathBuf::from("src/e2.rs"), PathBuf::from("src/lib.rs")]
    );
    drop(engine);
    assert_eq!(
        w.dir()
            .read_plan()
            .unwrap()
            .unwrap()
            .element(&w.ids[1])
            .unwrap()
            .files
            .len(),
        2,
        "plan.toml holds the new scope, which the hooks read"
    );
    let resume = &driver.replay.calls()[0];
    assert_eq!(resume.resume.as_deref(), Some("agent-2"));
    assert!(
        resume
            .task
            .prompt
            .contains("added src/lib.rs to this element's files")
    );
}

#[test]
fn an_agent_that_dies_mid_turn_leaves_the_cursor_where_it_was() {
    let w = world(1, false, "true");
    let died: String = element_turn("agent-1")
        .lines()
        .filter(|l| !l.contains("\"result\""))
        .collect::<Vec<_>>()
        .join("\n");
    let mut driver = w.driver(vec![died]);
    let mut engine = w.engine(&mut driver, &CLEAN);
    assert!(matches!(
        engine.run(),
        Err(Error::Driver(DriverError::NoResult))
    ));
    drop(engine);
    let s = w.dir().read_session().unwrap().unwrap();
    assert_eq!(s.state, SessionState::Running);
    assert_eq!(s.cursor.todo, w.ids);
    assert_eq!(
        s.agent(&w.ids[0]).unwrap().session_id,
        "agent-1",
        "known before the first event"
    );

    // The next run resumes that driver session.
    let mut driver = w.driver(vec![
        element_turn("agent-1"),
        judge_turn("judge", &pass_all(&w.ids)),
    ]);
    let mut engine = w.engine(&mut driver, &CLEAN);
    assert_eq!(engine.run().unwrap(), SessionState::Done);
    drop(engine);
    assert_eq!(driver.replay.calls()[0].resume.as_deref(), Some("agent-1"));
}

#[test]
fn the_context_pack_reaches_the_element_and_the_judge() {
    let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/arch-fixtures/golden/smallsvc/facts.json");
    let facts: Facts =
        serde_json::from_str(&std::fs::read_to_string(&golden).unwrap_or_else(|e| {
            panic!("{}: {e}; run scripts/fetch-fixtures.sh", golden.display())
        }))
        .unwrap();
    let project = FakeProject {
        blocking: 0,
        facts: Some(facts),
    };
    let w = world(1, false, "true");
    let mut driver = w.driver(vec![
        element_turn("agent-1"),
        judge_turn("judge", &pass_all(&w.ids)),
    ]);
    let mut engine = w.engine(&mut driver, &project);
    assert_eq!(engine.run().unwrap(), SessionState::Done);
    drop(engine);
    let calls = driver.replay.calls();
    let element_pack = calls[0].context.pack.as_ref().unwrap();
    assert!(element_pack.ends_with(format!(".arch/cache/driver/{}/pack.md", w.ids[0])));
    let judge_pack = calls[1].context.pack.as_ref().unwrap();
    assert!(judge_pack.ends_with(".arch/cache/driver/judge/pack.md"));
    assert!(!std::fs::read_to_string(element_pack).unwrap().is_empty());

    // Without facts the run goes on without a pack, and says so in the thread.
    let w = world(1, false, "true");
    let mut driver = w.driver(vec![
        element_turn("agent-1"),
        judge_turn("judge", &pass_all(&w.ids)),
    ]);
    let mut engine = w.engine(&mut driver, &CLEAN);
    assert_eq!(engine.run().unwrap(), SessionState::Done);
    drop(engine);
    assert_eq!(driver.replay.calls()[0].context.pack, None);
    assert!(
        w.dir()
            .read_thread()
            .unwrap()
            .iter()
            .any(|t| matches!(&t.event,
        ThreadEvent::Text { text } if text.starts_with("no context pack for")))
    );
}

#[test]
fn a_session_that_is_not_running_does_nothing() {
    let w = world(1, false, "true");
    let mut s = w.dir().read_session().unwrap().unwrap();
    s.state = SessionState::Done;
    w.dir().write_session(&s).unwrap();
    let mut driver = w.driver(vec![]);
    let mut engine = w.engine(&mut driver, &CLEAN);
    assert_eq!(engine.run().unwrap(), SessionState::Done);
    assert!(matches!(
        engine.answer(vec![]),
        Err(Error::IllegalTransition {
            from: SessionState::Done,
            ..
        })
    ));
    drop(engine);
    assert!(driver.replay.calls().is_empty());
}
