//! `arch session …` end to end (#48, milestone 5's acceptance): the binary on a temp copy of
//! smallsvc with a local bare `origin`, the acting replay driver (`ARCH_DRIVER=replay:<dir>`) and
//! the fake forge (`ARCH_FORGE=fake:<dir>`).
//!
//! ```text
//! new --plan ─▶ accept --delegate ─▶ E1…E4 (Write + mcp__arch__commit, through arch hook / arch mcp)
//!            ─▶ gate (test -f …, arch check, judge 4/4) ─▶ pr ─▶ merge ─▶ refs/notes/arch
//! ```
//!
//! The fake forge records GitHub's answers; it does not merge. The test makes the "merge commit"
//! on `origin`'s main itself and records its sha as the merge's answer.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

// ---------------------------------------------------------------- the world

struct World {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    repo: PathBuf,
    origin: PathBuf,
}

const FILES: [&str; 4] = [
    "src/domain/refund.rs",
    "src/ports/refunds.rs",
    "src/app/refund.rs",
    "src/adapters/memory/refunds.rs",
];

/// smallsvc copied into a fresh git repo whose `origin` is a local bare repository.
fn world() -> World {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/arch-fixtures/repos/smallsvc");
    assert!(
        src.join("Cargo.toml").is_file(),
        "{} is missing: run scripts/fetch-fixtures.sh",
        src.display()
    );
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let repo = root.join("smallsvc");
    copy_dir(&src, &repo);
    let origin = root.join("origin.git");
    git(&root, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
    git(&repo, &["init", "-q", "-b", "main"]);
    identity(&repo);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    git(
        &repo,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&repo, &["push", "-q", "-u", "origin", "main"]);
    std::fs::create_dir_all(root.join("wt")).unwrap();
    World {
        _tmp: tmp,
        root,
        repo,
        origin,
    }
}

impl World {
    /// `arch session <args>` in the repo, with the test's settings in the environment.
    fn arch(&self, args: &[&str], env: &[(&str, &Path)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_arch"));
        cmd.arg("session")
            .args(args)
            .current_dir(&self.repo)
            .env("ARCH_WORKTREE_PARENT", self.root.join("wt"))
            .env("ARCH_TEST_COMMAND", gate_command())
            .env_remove("ARCH_DRIVER")
            .env_remove("ARCH_FORGE");
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    fn ok(&self, args: &[&str], env: &[(&str, &Path)]) -> String {
        let out = self.arch(args, env);
        assert!(
            out.status.success(),
            "arch session {args:?} exited {:?}\nstdout:\n{}\nstderr:\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn status(&self, id: &str) -> Value {
        serde_json::from_str(&self.ok(&["status", id, "--format", "json"], &[])).unwrap()
    }

    fn dir(&self, name: &str) -> PathBuf {
        let d = self.root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A plan file with the four elements, all independent: one group.
    fn plan_file(&self) -> PathBuf {
        let intentions = [
            "add Refund to the domain",
            "a Refunds port",
            "the refund use case",
            "an in-memory Refunds adapter",
        ];
        let text: String = intentions
            .iter()
            .zip(FILES)
            .map(|(i, f)| {
                format!("[[element]]\nintention = \"{i}\"\nsite = \"{f}\"\nfiles = [\"{f}\"]\n\n")
            })
            .collect();
        let p = self.root.join("plan.toml");
        std::fs::write(&p, text).unwrap();
        p
    }

    /// `arch session new --plan`: the id.
    fn new_session(&self) -> String {
        let plan = self.plan_file();
        let out = self.ok(
            &["new", "add a refund flow", "--plan", plan.to_str().unwrap()],
            &[],
        );
        assert!(out.contains("4 element(s), 1 group(s)"), "{out}");
        assert!(out.contains("draft in the cache"), "{out}");
        out.split_whitespace().nth(1).unwrap().to_string()
    }

    fn element_ids(&self, id: &str) -> Vec<String> {
        self.status(id)["plan"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap().to_string())
            .collect()
    }
}

/// The gate's command: the four files exist. (smallsvc's own `cargo test` pulls axum and sqlx,
/// too slow and networked for the default CI; `arch check` and the judge run for real.)
fn gate_command() -> String {
    FILES
        .iter()
        .map(|f| format!("test -f {f}"))
        .collect::<Vec<_>>()
        .join(" && ")
}

// ---------------------------------------------------------------- recorded turns

fn turn(sid: &str, calls: &[(&str, Value)], structured: Option<Value>) -> String {
    let mut out = vec![
        json!({ "type": "system", "subtype": "init", "session_id": sid,
        "model": "replay", "tools": [] }),
    ];
    for (i, (name, input)) in calls.iter().enumerate() {
        let id = format!("toolu_{i}");
        out.push(
            json!({ "type": "assistant", "session_id": sid, "parent_tool_use_id": null,
            "message": { "role": "assistant", "content": [{ "type": "tool_use", "id": id,
                "name": name, "input": input }] } }),
        );
        out.push(
            json!({ "type": "user", "session_id": sid, "parent_tool_use_id": null,
            "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": id,
                "content": "ok" }] } }),
        );
    }
    let mut result = json!({ "type": "result", "subtype": "success", "is_error": false,
        "session_id": sid, "result": "done", "num_turns": 1 });
    if let Some(s) = structured {
        result["structured_output"] = s;
    }
    out.push(result);
    out.iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Element `n`'s turn: write its file, commit through arch's tool.
fn element_turn(n: usize) -> String {
    let file = FILES[n - 1];
    turn(
        &format!("agent-{n}"),
        &[
            (
                "Write",
                json!({ "file_path": file, "content": format!("// {file}\npub struct Refund{n};\n") }),
            ),
            (
                "mcp__arch__commit",
                json!({ "type": "feat", "summary": format!("refund step {n}") }),
            ),
        ],
        None,
    )
}

fn judge_turn(ids: &[String], pass: bool) -> String {
    let verdicts: Vec<Value> = ids
        .iter()
        .map(|id| {
            json!({ "element": id, "verdict": if pass { "pass" } else { "fail" },
            "reason": if pass { "realized as intended" } else { "the refund is never persisted" } })
        })
        .collect();
    turn("judge-1", &[], Some(json!({ "verdicts": verdicts })))
}

fn replay_dir(w: &World, name: &str, turns: &[String]) -> PathBuf {
    let dir = w.dir(name);
    for (i, t) in turns.iter().enumerate() {
        std::fs::write(dir.join(format!("{:02}.jsonl", i + 1)), t).unwrap();
    }
    dir
}

// ---------------------------------------------------------------- the fake forge

fn pull(number: u64, id: &str, sha: &str, state: &str, merged: bool) -> Value {
    json!({ "number": number, "html_url": format!("https://github.com/fake/repo/pull/{number}"),
        "title": "add a refund flow", "state": state, "draft": false,
        "merged_at": if merged { json!("2026-10-04T12:00:00Z") } else { Value::Null },
        "head": { "ref": format!("arch/{id}"), "sha": sha },
        "base": { "ref": "main", "sha": "0000000" } })
}

fn forge_dir(w: &World, name: &str, recorded: Value) -> PathBuf {
    let dir = w.dir(name);
    std::fs::write(dir.join("recorded.json"), recorded.to_string()).unwrap();
    dir
}

fn pulls_path(id: &str) -> String {
    format!("/repos/fake/repo/pulls?head=fake:arch/{id}&state=all")
}

// ---------------------------------------------------------------- the acceptance test

#[test]
fn one_delegated_session_end_to_end_then_merge_archives_it() {
    let w = world();
    let id = w.new_session();
    assert_eq!(w.status(&id)["state"], "planning");
    let ids = w.element_ids(&id);
    assert_eq!(ids.len(), 4);

    // Accept · delegate: four acting element turns, then the judge.
    let mut turns: Vec<String> = (1..=4).map(element_turn).collect();
    turns.push(judge_turn(&ids, true));
    let replay = replay_dir(&w, "replay", &turns);
    let out = w.ok(
        &["accept", &id, "--delegate"],
        &[(
            "ARCH_DRIVER",
            &PathBuf::from(format!("replay:{}", replay.display())),
        )],
    );
    let worktree = w.root.join("wt/smallsvc-w1");
    assert!(
        out.contains(&format!(
            "accepted · branch arch/{id} · worktree {}",
            worktree.display()
        )),
        "{out}"
    );
    assert!(
        out.contains("running group 1/1 · E1 E2 E3 E4 done"),
        "{out}"
    );
    assert!(out.contains("judge 4/4 pass"), "{out}");
    assert!(
        out.contains(&format!("done · arch session pr {id}")),
        "{out}"
    );

    let status = w.status(&id);
    assert_eq!(status["state"], "done");
    assert_eq!(status["gates"].as_array().unwrap().len(), 1, "one gate");
    for f in FILES {
        assert!(worktree.join(f).is_file(), "{f} written through the hooks");
    }

    // Four element commits, one per element, with both trailers.
    let base = git(&w.repo, &["rev-parse", "main"]);
    let log = git(
        &worktree,
        &["log", "--format=%H%x1f%B%x1e", &format!("{base}..HEAD")],
    );
    let element_commits: Vec<&str> = log
        .split('\x1e')
        .filter(|c| c.contains("Arch-Element: "))
        .collect();
    assert_eq!(element_commits.len(), 4, "{log}");
    for (n, element) in ids.iter().enumerate() {
        let c = element_commits
            .iter()
            .find(|c| c.contains(&format!("Arch-Element: {element}")))
            .unwrap_or_else(|| panic!("no commit for {element}:\n{log}"));
        assert!(c.contains(&format!("Arch-Session: {id}")), "{c}");
        assert!(c.contains(&format!("refund step {}", n + 1)), "{c}");
        assert!(c.contains("Co-authored-by: Claude"), "{c}");
    }

    // pr: the branch reaches origin, the request is opened, the session is in review.
    let head = git(&worktree, &["rev-parse", "HEAD"]);
    let forge = forge_dir(
        &w,
        "forge-pr",
        json!([
            { "method": "GET", "path": pulls_path(&id), "status": 200, "json": [] },
            { "method": "POST", "path": "/repos/fake/repo/pulls", "status": 201,
              "json": pull(1, &id, &head, "open", false) },
        ]),
    );
    let fake = |dir: &Path| PathBuf::from(format!("fake:{}", dir.display()));
    let out = w.ok(&["pr", &id], &[("ARCH_FORGE", &fake(&forge))]);
    assert!(
        out.contains("PR #1 · https://github.com/fake/repo/pull/1 · opened"),
        "{out}"
    );
    assert_eq!(
        git(&w.origin, &["rev-parse", &format!("arch/{id}")]),
        head,
        "create_pr pushed the branch"
    );
    assert_eq!(w.status(&id)["state"], "in-review");

    // merge: GitHub's merge is recorded; its commit is made on origin's main by the test.
    let merge_sha = fake_merge_commit(&w, &id);
    let forge = forge_dir(
        &w,
        "forge-merge",
        json!([
            { "method": "GET", "path": pulls_path(&id), "status": 200,
              "json": [pull(1, &id, &head, "open", false)] },
            { "method": "GET", "path": format!("/repos/fake/repo/commits/{head}/check-runs?per_page=100"),
              "status": 200, "json": { "total_count": 0, "check_runs": [] } },
            { "method": "GET", "path": format!("/repos/fake/repo/commits/{head}/status?per_page=100"),
              "status": 200, "json": { "state": "pending", "statuses": [] } },
            { "method": "GET", "path": "/repos/fake/repo", "status": 200,
              "json": { "allow_merge_commit": true, "allow_squash_merge": false, "allow_rebase_merge": false } },
            { "method": "PUT", "path": "/repos/fake/repo/pulls/1/merge", "status": 200,
              "json": { "sha": merge_sha, "merged": true, "message": "Pull Request successfully merged" } },
        ]),
    );
    let out = w.ok(&["merge", &id], &[("ARCH_FORGE", &fake(&forge))]);
    assert!(out.contains("merged PR #1 (merge)"), "{out}");
    assert!(out.contains("archived to refs/notes/arch"), "{out}");

    // The branch on origin no longer holds the session folder: main's tree never will.
    let folder = format!(".arch/sessions/{id}");
    assert_eq!(
        git(
            &w.origin,
            &[
                "ls-tree",
                "-d",
                "--name-only",
                &format!("arch/{id}"),
                &folder
            ]
        ),
        ""
    );
    // The archive is the note on the merge commit.
    let note = git(&w.repo, &["notes", "--ref", "arch", "show", &merge_sha]);
    assert!(note.contains(&format!("session = \"{id}\"")), "{note}");
    for f in ["plan.toml", "session.toml", "thread.jsonl"] {
        assert!(note.contains(&format!("path = \"{f}\"")), "{f} archived");
    }
    assert!(note.contains("path = \"records/"), "records archived");
    // Worktree and branch gone.
    assert!(!worktree.exists(), "worktree removed");
    assert!(
        git_status(
            &w.repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/arch/{id}")
            ]
        )
        .is_err(),
        "branch removed"
    );
    // And the session is still there, from the notes.
    let status = w.status(&id);
    assert_eq!(status["state"], "archived");
    assert_eq!(status["archived_on"], merge_sha.as_str());
    assert_eq!(status["plan"]["elements"].as_array().unwrap().len(), 4);
}

/// The commit a merge would leave on origin's main.
fn fake_merge_commit(w: &World, id: &str) -> String {
    let clone = w.root.join("merger");
    git(
        &w.root,
        &["clone", "-q", w.origin.to_str().unwrap(), "merger"],
    );
    identity(&clone);
    git(
        &clone,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            &format!("Merge pull request #1 from fake/arch/{id}"),
        ],
    );
    git(&clone, &["push", "-q", "origin", "main"]);
    git(&clone, &["rev-parse", "HEAD"])
}

// ---------------------------------------------------------------- helpers

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "target" || name == ".git" || name == "cache" {
            continue;
        }
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to.join(&name));
        } else {
            std::fs::copy(entry.path(), to.join(&name)).unwrap();
        }
    }
}

fn identity(repo: &Path) {
    git(repo, &["config", "user.name", "Person"]);
    git(repo, &["config", "user.email", "person@example.com"]);
}

fn git_status(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    git_status(dir, args).unwrap_or_else(|e| panic!("git {args:?}: {e}"))
}
