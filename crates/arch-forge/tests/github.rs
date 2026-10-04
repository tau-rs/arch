//! The GitHub adapter against recorded GitHub JSON (`tests/fixtures/github`, no network).

use std::path::{Path, PathBuf};
use std::process::Command;

use arch_forge::{
    CheckState, Forge, ForgeError, GitHub, Method, NewRequest, Recorded, RepoRef, Request,
    RequestState, ReviewState, Reviewer, Strategy, summary,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/github")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

fn repo() -> RepoRef {
    RepoRef::new("tau-rs", "arch")
}

fn github(recorded: Recorded) -> GitHub<Recorded> {
    GitHub::new(repo(), PathBuf::from("."), recorded)
}

/// The requests the double received, as `METHOD path`.
fn sent(forge: &GitHub<Recorded>) -> Vec<String> {
    forge
        .transport()
        .sent()
        .iter()
        .map(|r| format!("{} {}", r.method, r.path))
        .collect()
}

fn pr_70() -> Request {
    let forge = github(Recorded::new().on(
        Method::Get,
        "/repos/tau-rs/arch/pulls?head=tau-rs:LEBOCQTitouan/facts-key-59-recompute&state=all",
        200,
        fixture("pulls-merged.json"),
    ));
    forge
        .pr("LEBOCQTitouan/facts-key-59-recompute")
        .unwrap()
        .unwrap()
}

#[test]
fn the_adapter_speaks_the_forges_words() {
    let forge = github(Recorded::new());
    assert_eq!(
        (forge.request_word(), forge.checks_word()),
        ("PR", "checks")
    );
}

#[test]
fn pr_finds_the_branchs_request_merged_or_not() {
    let pr = pr_70();
    assert_eq!(
        pr,
        Request {
            number: 70,
            url: "https://github.com/tau-rs/arch/pull/70".into(),
            title: pr.title.clone(),
            head: "LEBOCQTitouan/facts-key-59-recompute".into(),
            base: "main".into(),
            head_sha: "f4a03559aa24d6e276e15ae37f8339b6d6dba2a5".into(),
            draft: false,
            state: RequestState::Merged,
        }
    );
}

#[test]
fn pr_is_none_when_the_branch_has_no_request() {
    let forge = github(Recorded::new().on(
        Method::Get,
        "/repos/tau-rs/arch/pulls?head=tau-rs:arch/3c9e1f0a&state=all",
        200,
        fixture("pulls-none.json"),
    ));
    assert_eq!(forge.pr("arch/3c9e1f0a").unwrap(), None);
    assert_eq!(
        sent(&forge),
        ["GET /repos/tau-rs/arch/pulls?head=tau-rs:arch/3c9e1f0a&state=all"]
    );
}

#[test]
fn pr_escapes_the_branch_in_the_query() {
    let forge = github(Recorded::new().on(
        Method::Get,
        "/repos/tau-rs/arch/pulls?head=tau-rs:a%26b%23c&state=all",
        200,
        fixture("pulls-none.json"),
    ));
    assert_eq!(forge.pr("a&b#c").unwrap(), None);
}

fn checks_for(sha: &str, runs: &str, status: &str) -> Vec<arch_forge::Check> {
    let forge = github(
        Recorded::new()
            .on(
                Method::Get,
                &format!("/repos/tau-rs/arch/commits/{sha}/check-runs?per_page=100"),
                200,
                fixture(runs),
            )
            .on(
                Method::Get,
                &format!("/repos/tau-rs/arch/commits/{sha}/status?per_page=100"),
                200,
                fixture(status),
            ),
    );
    let pr = Request {
        head_sha: sha.into(),
        ..pr_70()
    };
    forge.checks(&pr).unwrap()
}

#[test]
fn checks_passed() {
    let checks = checks_for("abc", "check-runs-passed.json", "status-none.json");
    let names: Vec<_> = checks.iter().map(|c| (c.name.as_str(), c.state)).collect();
    assert_eq!(
        names,
        [
            (
                "budgets (ADR 0026): first index, one-file recompute",
                CheckState::Passed
            ),
            (
                "dependency direction (arrows go one way)",
                CheckState::Passed
            ),
            ("check", CheckState::Passed),
            ("arch check on zero2prod (milestone 4)", CheckState::Passed),
        ]
    );
    assert!(
        checks[2]
            .url
            .as_deref()
            .unwrap()
            .starts_with("https://github.com/tau-rs/arch/actions/runs/")
    );
    // No statuses at all reads as passed, although GitHub's combined `state` says pending.
    assert_eq!(summary(&checks), CheckState::Passed);
}

#[test]
fn checks_failed() {
    let checks = checks_for("abc", "check-runs-failed.json", "status-none.json");
    assert_eq!(
        checks.iter().find(|c| c.name == "check").unwrap().state,
        CheckState::Failed
    );
    assert_eq!(summary(&checks), CheckState::Failed);
}

#[test]
fn checks_pending() {
    let checks = checks_for("abc", "check-runs-pending.json", "status-none.json");
    assert_eq!(checks[0].state, CheckState::Pending);
    assert_eq!(summary(&checks), CheckState::Pending);
}

#[test]
fn checks_include_commit_statuses_from_other_ci() {
    let checks = checks_for("abc", "check-runs-passed.json", "status-failed.json");
    let external = checks.iter().find(|c| c.name == "ci/external").unwrap();
    assert_eq!(external.state, CheckState::Failed);
    assert_eq!(
        external.url.as_deref(),
        Some("https://ci.example.com/1000/output")
    );
    assert_eq!(summary(&checks), CheckState::Failed);
}

#[test]
fn summary_of_nothing_is_passed_and_skipped_never_fails() {
    use arch_forge::Check;
    let c = |state| Check {
        name: "x".into(),
        state,
        url: None,
    };
    assert_eq!(summary(&[]), CheckState::Passed);
    assert_eq!(
        summary(&[c(CheckState::Skipped), c(CheckState::Passed)]),
        CheckState::Passed
    );
    assert_eq!(
        summary(&[c(CheckState::Pending), c(CheckState::Failed)]),
        CheckState::Failed
    );
}

fn reviewers_for(requested: &str, reviews: &str) -> Vec<Reviewer> {
    let pr = pr_70();
    let forge = github(
        Recorded::new()
            .on(
                Method::Get,
                "/repos/tau-rs/arch/pulls/70/requested_reviewers",
                200,
                fixture(requested),
            )
            .on(
                Method::Get,
                "/repos/tau-rs/arch/pulls/70/reviews?per_page=100",
                200,
                fixture(reviews),
            ),
    );
    forge.reviewers(&pr).unwrap()
}

#[test]
fn reviewers_take_each_persons_latest_verdict() {
    let r = |name: &str, state| Reviewer {
        name: name.into(),
        team: false,
        state,
    };
    assert_eq!(
        reviewers_for("requested-reviewers-none.json", "reviews-mixed.json"),
        [
            r("A4-Tacks", ReviewState::Approved),
            r("Young-Flash", ReviewState::Commented),
            r("ChayimFriedman2", ReviewState::ChangesRequested),
        ]
    );
}

#[test]
fn a_re_requested_reviewer_is_requested_again() {
    let r = |name: &str, state| Reviewer {
        name: name.into(),
        team: false,
        state,
    };
    assert_eq!(
        reviewers_for("requested-reviewers.json", "reviews.json"),
        [
            r("epage", ReviewState::Requested),
            r("hirehamir", ReviewState::Commented),
            r("Samielakkad", ReviewState::Commented),
            r("0xPoe", ReviewState::Commented),
        ]
    );
}

#[test]
fn strategies_are_read_from_the_repo() {
    let mut repo_json = fixture("repo.json");
    let forge =
        github(Recorded::new().on(Method::Get, "/repos/tau-rs/arch", 200, repo_json.clone()));
    assert_eq!(
        forge.strategies().unwrap(),
        [Strategy::Merge, Strategy::Squash, Strategy::Rebase]
    );

    repo_json["allow_merge_commit"] = json!(false);
    repo_json["allow_rebase_merge"] = json!(false);
    let forge = github(Recorded::new().on(Method::Get, "/repos/tau-rs/arch", 200, repo_json));
    assert_eq!(forge.strategies().unwrap(), [Strategy::Squash]);
}

#[test]
fn merge_sends_the_strategy_and_the_head_it_saw() {
    let pr = pr_70();
    let forge = github(Recorded::new().on(
        Method::Put,
        "/repos/tau-rs/arch/pulls/70/merge",
        200,
        fixture("merge-ok.json"),
    ));
    let merged = forge.merge(&pr, Strategy::Squash).unwrap();
    assert_eq!(merged.sha, "6dcb09b5b57875f334f61aebed695e2e4193db5e");
    let body = forge.transport().sent()[0].body.clone().unwrap();
    assert_eq!(
        body,
        json!({"merge_method": "squash", "sha": "f4a03559aa24d6e276e15ae37f8339b6d6dba2a5"})
    );
}

#[test]
fn merge_with_a_strategy_the_repo_does_not_allow_is_rejected() {
    let pr = pr_70();
    let forge = github(Recorded::new().on(
        Method::Put,
        "/repos/tau-rs/arch/pulls/70/merge",
        405,
        fixture("merge-405.json"),
    ));
    match forge.merge(&pr, Strategy::Merge) {
        Err(ForgeError::Rejected {
            status: 405,
            message,
        }) => {
            assert_eq!(message, "Merge commits are not allowed on this repository.")
        }
        other => panic!("expected a 405 rejection, got {other:?}"),
    }
}

#[test]
fn an_unrecorded_request_fails_loudly() {
    let forge = github(Recorded::new());
    let err = forge.strategies().unwrap_err();
    assert!(matches!(err, ForgeError::Transport(_)), "{err:?}");
    assert!(err.to_string().contains("GET /repos/tau-rs/arch"), "{err}");
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// A clone whose `origin` is a local bare repository, on branch `arch/3c9e1f0a` with one commit.
fn clone_with_bare_origin() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let origin = tmp.path().join("origin.git");
    let work = tmp.path().join("work");
    git(tmp.path(), &["init", "-q", "--bare", "origin.git"]);
    git(tmp.path(), &["init", "-q", "work"]);
    git(&work, &["config", "user.email", "arch@example.com"]);
    git(&work, &["config", "user.name", "arch"]);
    git(
        &work,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&work, &["checkout", "-q", "-b", "arch/3c9e1f0a"]);
    git(
        &work,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "feat(domain): add Refund",
        ],
    );
    (tmp, origin, work)
}

#[test]
fn create_pr_pushes_the_head_then_opens_the_request() {
    let (_tmp, origin, work) = clone_with_bare_origin();
    let forge = GitHub::new(
        repo(),
        work.clone(),
        Recorded::new().on(
            Method::Post,
            "/repos/tau-rs/arch/pulls",
            201,
            fixture("pull-created.json"),
        ),
    );
    let new = NewRequest {
        head: "arch/3c9e1f0a".into(),
        base: "main".into(),
        title: "feat(forge): Forge trait and the GitHub adapter (#45)".into(),
        body: "Closes #45".into(),
        draft: true,
    };
    let pr = forge.create_pr(&new).unwrap();

    assert_eq!(
        git(&origin, &["rev-parse", "arch/3c9e1f0a"]),
        git(&work, &["rev-parse", "HEAD"])
    );
    assert_eq!(
        git(
            &work,
            &["rev-parse", "--abbrev-ref", "arch/3c9e1f0a@{upstream}"]
        ),
        "origin/arch/3c9e1f0a"
    );
    assert_eq!(
        (pr.number, pr.draft, pr.state),
        (72, true, RequestState::Open)
    );
    let body = forge.transport().sent()[0].body.clone().unwrap();
    assert_eq!(
        body,
        json!({"head": "arch/3c9e1f0a", "base": "main", "title": new.title, "body": "Closes #45", "draft": true})
    );
}

#[test]
fn create_pr_does_not_open_a_request_when_the_push_fails() {
    let (_tmp, _origin, work) = clone_with_bare_origin();
    let forge = GitHub::new(repo(), work, Recorded::new());
    let new = NewRequest {
        head: "no-such-branch".into(),
        base: "main".into(),
        ..Default::default()
    };
    let err = forge.create_pr(&new).unwrap_err();
    assert!(matches!(err, ForgeError::Push { .. }), "{err:?}");
    assert!(forge.transport().sent().is_empty());
}

#[test]
fn push_sets_the_upstream() {
    let (_tmp, origin, work) = clone_with_bare_origin();
    let forge = GitHub::new(repo(), work.clone(), Recorded::new());
    forge.push("arch/3c9e1f0a").unwrap();
    assert_eq!(
        git(&origin, &["rev-parse", "arch/3c9e1f0a"]),
        git(&work, &["rev-parse", "HEAD"])
    );
    assert!(forge.transport().sent().is_empty());
}

#[test]
fn owner_and_repo_come_from_the_remote_url() {
    for url in [
        "https://github.com/tau-rs/arch.git",
        "https://github.com/tau-rs/arch",
        "https://x-access-token:secret@github.com/tau-rs/arch.git",
        "git@github.com:tau-rs/arch.git",
        "ssh://git@github.com/tau-rs/arch.git",
        "ssh://git@github.com:22/tau-rs/arch",
    ] {
        assert_eq!(RepoRef::from_remote(url), Some(repo()), "{url}");
    }
    for url in [
        "https://gitlab.com/tau-rs/arch.git",
        "/tmp/origin.git",
        "git@github.com:arch.git",
    ] {
        assert_eq!(RepoRef::from_remote(url), None, "{url}");
    }
}

#[test]
fn a_clone_whose_origin_is_not_github_is_refused() {
    let (_tmp, _origin, work) = clone_with_bare_origin();
    assert!(matches!(
        RepoRef::of_origin(&work),
        Err(ForgeError::NotGitHub { .. })
    ));
    git(
        &work,
        &[
            "remote",
            "set-url",
            "origin",
            "git@github.com:tau-rs/arch.git",
        ],
    );
    assert_eq!(RepoRef::of_origin(&work).unwrap(), repo());
}

#[test]
fn the_double_loads_a_directory_for_the_fake_forge() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/github/repo.json"),
        tmp.path().join("repo.json"),
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("recorded.json"),
        r#"[
          {"method": "GET", "path": "/repos/tau-rs/arch", "status": 200, "file": "repo.json"},
          {"method": "PUT", "path": "/repos/tau-rs/arch/pulls/70/merge", "status": 200, "json": {"sha": "abc", "merged": true}}
        ]"#,
    )
    .unwrap();
    let forge = github(Recorded::from_dir(tmp.path()).unwrap());
    assert_eq!(forge.strategies().unwrap().len(), 3);
    assert_eq!(forge.merge(&pr_70(), Strategy::Merge).unwrap().sha, "abc");
}

#[test]
fn the_double_answers_the_same_request_in_recorded_order() {
    let forge = github(
        Recorded::new()
            .on(
                Method::Get,
                "/repos/tau-rs/arch/pulls?head=tau-rs:b&state=all",
                200,
                fixture("pulls-none.json"),
            )
            .on(
                Method::Get,
                "/repos/tau-rs/arch/pulls?head=tau-rs:b&state=all",
                200,
                fixture("pulls-merged.json"),
            ),
    );
    assert_eq!(forge.pr("b").unwrap(), None);
    assert_eq!(forge.pr("b").unwrap().unwrap().number, 70);
}

#[test]
fn a_comment_after_a_verdict_keeps_the_verdict_and_a_dismissal_drops_it() {
    let pr = pr_70();
    let review = |login: &str, state: &str| json!({"user": {"login": login}, "state": state});
    let forge = github(
        Recorded::new()
            .on(
                Method::Get,
                "/repos/tau-rs/arch/pulls/70/requested_reviewers",
                200,
                json!({"users": [], "teams": [{"slug": "reviewers"}]}),
            )
            .on(
                Method::Get,
                "/repos/tau-rs/arch/pulls/70/reviews?per_page=100",
                200,
                json!([
                    review("ana", "APPROVED"),
                    review("ana", "COMMENTED"),
                    review("bo", "CHANGES_REQUESTED"),
                    review("bo", "DISMISSED"),
                    review("cy", "PENDING"),
                    {"user": null, "state": "APPROVED"},
                ]),
            ),
    );
    let r = |name: &str, team, state| Reviewer {
        name: name.into(),
        team,
        state,
    };
    assert_eq!(
        forge.reviewers(&pr).unwrap(),
        [
            r("ana", false, ReviewState::Approved),
            r("bo", false, ReviewState::Commented),
            r("tau-rs/reviewers", true, ReviewState::Requested),
        ]
    );
}
