//! The GitHub adapter: REST through a [`Transport`], push through `git`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{
    Check, CheckState, Forge, ForgeError, HttpRequest, Merged, Method, NewRequest, Request,
    RequestState, ReviewState, Reviewer, Strategy, Transport, Ureq, resolve_token,
};

/// A repository on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    /// The user or organisation.
    pub owner: String,
    /// The repository's name.
    pub name: String,
}

impl RepoRef {
    /// `owner/name`.
    pub fn new(owner: &str, name: &str) -> Self {
        RepoRef {
            owner: owner.into(),
            name: name.into(),
        }
    }

    /// The repository a GitHub remote URL points at: `https://github.com/o/r(.git)`,
    /// `git@github.com:o/r(.git)` or `ssh://git@github.com(:port)/o/r(.git)`. `None` otherwise.
    pub fn from_remote(url: &str) -> Option<Self> {
        let (host, path) = match url.split_once("://") {
            Some((_, rest)) => {
                let (authority, path) = rest.split_once('/')?;
                let host = authority.rsplit('@').next()?;
                (host.split(':').next()?, path)
            }
            None => {
                let (host, path) = url.split_once(':')?;
                if host.contains('/') {
                    return None;
                }
                (host.rsplit('@').next()?, path)
            }
        };
        if !host.eq_ignore_ascii_case("github.com") {
            return None;
        }
        let path = path.trim_end_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        match path.split('/').collect::<Vec<_>>()[..] {
            [owner, name] if !owner.is_empty() && !name.is_empty() => {
                Some(RepoRef::new(owner, name))
            }
            _ => None,
        }
    }

    /// The repository `origin` points at in the clone or worktree `dir`.
    pub fn of_origin(dir: &Path) -> Result<Self, ForgeError> {
        let out = Command::new("git")
            .current_dir(dir)
            .args(["remote", "get-url", "origin"])
            .output()
            .map_err(|e| ForgeError::NotGitHub {
                remote: format!("git remote get-url origin: {e}"),
            })?;
        if !out.status.success() {
            return Err(ForgeError::NotGitHub {
                remote: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
        RepoRef::from_remote(&url).ok_or(ForgeError::NotGitHub { remote: url })
    }
}

/// The GitHub adapter. `T` carries the REST calls: [`Ureq`] in production, `Recorded` in tests.
#[derive(Debug)]
pub struct GitHub<T = Ureq> {
    repo: RepoRef,
    workdir: PathBuf,
    transport: T,
}

impl GitHub<Ureq> {
    /// The adapter for the clone or worktree `workdir`: owner and repo from its `origin`, the
    /// token from [`resolve_token`] with this process's environment.
    pub fn open(workdir: &Path) -> Result<Self, ForgeError> {
        let repo = RepoRef::of_origin(workdir)?;
        let token = resolve_token(|k| std::env::var(k).ok(), None)?;
        Ok(GitHub::new(repo, workdir.to_path_buf(), Ureq::new(token)))
    }
}

impl<T: Transport> GitHub<T> {
    /// The adapter for `repo`, pushing from `workdir`, over `transport`.
    pub fn new(repo: RepoRef, workdir: PathBuf, transport: T) -> Self {
        GitHub {
            repo,
            workdir,
            transport,
        }
    }

    /// The transport, for a test to read what was sent.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The repository.
    pub fn repo(&self) -> &RepoRef {
        &self.repo
    }

    /// Send, and decode a 2xx answer as `R`; any other status is [`ForgeError::Rejected`].
    fn call<R: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<R, ForgeError> {
        let path = format!("/repos/{}/{}{path}", self.repo.owner, self.repo.name);
        let response = self.transport.send(&HttpRequest {
            method,
            path: path.clone(),
            body,
        })?;
        if !(200..300).contains(&response.status) {
            return Err(ForgeError::Rejected {
                status: response.status,
                message: message(&response.body, response.status),
            });
        }
        serde_json::from_value(response.body)
            .map_err(|e| ForgeError::Decode(format!("{method} {path}: {e}")))
    }
}

/// GitHub's error message, with the validation details a 422 carries.
fn message(body: &Value, status: u16) -> String {
    let mut message = body["message"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("HTTP {status}"));
    if let Some(errors) = body["errors"].as_array() {
        let details: Vec<&str> = errors
            .iter()
            .filter_map(|e| e["message"].as_str())
            .collect();
        if !details.is_empty() {
            message = format!("{message}: {}", details.join("; "));
        }
    }
    message
}

/// Percent-encode a query value, keeping `/` (branch names) readable.
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Deserialize)]
struct PullJson {
    number: u64,
    html_url: String,
    title: String,
    state: String,
    #[serde(default)]
    draft: bool,
    merged_at: Option<String>,
    head: RefJson,
    base: RefJson,
}

#[derive(Deserialize)]
struct RefJson {
    #[serde(rename = "ref")]
    name: String,
    sha: String,
}

impl From<PullJson> for Request {
    fn from(p: PullJson) -> Self {
        let state = match (p.state.as_str(), &p.merged_at) {
            ("open", _) => RequestState::Open,
            (_, Some(_)) => RequestState::Merged,
            _ => RequestState::Closed,
        };
        Request {
            number: p.number,
            url: p.html_url,
            title: p.title,
            head: p.head.name,
            base: p.base.name,
            head_sha: p.head.sha,
            draft: p.draft,
            state,
        }
    }
}

#[derive(Deserialize)]
struct CheckRunsJson {
    check_runs: Vec<CheckRunJson>,
}

#[derive(Deserialize)]
struct CheckRunJson {
    name: String,
    status: String,
    conclusion: Option<String>,
    html_url: Option<String>,
}

#[derive(Deserialize)]
struct StatusesJson {
    statuses: Vec<StatusJson>,
}

#[derive(Deserialize)]
struct StatusJson {
    context: String,
    state: String,
    target_url: Option<String>,
}

#[derive(Deserialize)]
struct RequestedJson {
    users: Vec<LoginJson>,
    teams: Vec<TeamJson>,
}

#[derive(Deserialize)]
struct LoginJson {
    login: String,
}

#[derive(Deserialize)]
struct TeamJson {
    slug: String,
}

#[derive(Deserialize)]
struct ReviewJson {
    user: Option<LoginJson>,
    state: String,
}

#[derive(Deserialize)]
struct RepoJson {
    #[serde(default)]
    allow_merge_commit: bool,
    #[serde(default)]
    allow_squash_merge: bool,
    #[serde(default)]
    allow_rebase_merge: bool,
}

#[derive(Deserialize)]
struct MergedJson {
    sha: String,
}

impl<T: Transport> Forge for GitHub<T> {
    fn push(&self, branch: &str) -> Result<(), ForgeError> {
        let failed = |stderr: String| ForgeError::Push {
            branch: branch.into(),
            stderr,
        };
        let out = Command::new("git")
            .current_dir(&self.workdir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(["push", "-u", "origin", branch])
            .output()
            .map_err(|e| failed(e.to_string()))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(failed(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ))
        }
    }

    fn create_pr(&self, new: &NewRequest) -> Result<Request, ForgeError> {
        self.push(&new.head)?;
        let body = json!({"head": new.head, "base": new.base, "title": new.title, "body": new.body, "draft": new.draft});
        self.call::<PullJson>(Method::Post, "/pulls", Some(body))
            .map(Request::from)
    }

    fn pr(&self, branch: &str) -> Result<Option<Request>, ForgeError> {
        let path = format!(
            "/pulls?head={}:{}&state=all",
            encode(&self.repo.owner),
            encode(branch)
        );
        let pulls: Vec<PullJson> = self.call(Method::Get, &path, None)?;
        let mut requests: Vec<Request> = pulls.into_iter().map(Request::from).collect();
        let at = requests
            .iter()
            .position(|r| r.state == RequestState::Open)
            .unwrap_or(0);
        Ok((!requests.is_empty()).then(|| requests.swap_remove(at)))
    }

    fn checks(&self, pr: &Request) -> Result<Vec<Check>, ForgeError> {
        let sha = &pr.head_sha;
        let runs: CheckRunsJson = self.call(
            Method::Get,
            &format!("/commits/{sha}/check-runs?per_page=100"),
            None,
        )?;
        // The combined `state` reads `pending` when there are no statuses at all: use each one.
        let statuses: StatusesJson = self.call(
            Method::Get,
            &format!("/commits/{sha}/status?per_page=100"),
            None,
        )?;
        let runs = runs.check_runs.into_iter().map(|r| Check {
            state: match (r.status.as_str(), r.conclusion.as_deref()) {
                ("completed", Some("success")) => CheckState::Passed,
                ("completed", Some("neutral" | "skipped")) => CheckState::Skipped,
                ("completed", _) => CheckState::Failed,
                _ => CheckState::Pending,
            },
            name: r.name,
            url: r.html_url,
        });
        let statuses = statuses.statuses.into_iter().map(|s| Check {
            state: match s.state.as_str() {
                "success" => CheckState::Passed,
                "pending" => CheckState::Pending,
                _ => CheckState::Failed,
            },
            name: s.context,
            url: s.target_url,
        });
        Ok(runs.chain(statuses).collect())
    }

    fn reviewers(&self, pr: &Request) -> Result<Vec<Reviewer>, ForgeError> {
        let n = pr.number;
        let requested: RequestedJson = self.call(
            Method::Get,
            &format!("/pulls/{n}/requested_reviewers"),
            None,
        )?;
        let reviews: Vec<ReviewJson> = self.call(
            Method::Get,
            &format!("/pulls/{n}/reviews?per_page=100"),
            None,
        )?;

        let person = |name: String, state| Reviewer {
            name,
            team: false,
            state,
        };
        let mut out: Vec<Reviewer> = requested
            .users
            .into_iter()
            .map(|u| person(u.login, ReviewState::Requested))
            .collect();
        let mut at: HashMap<String, usize> = out
            .iter()
            .enumerate()
            .map(|(i, r)| (r.name.clone(), i))
            .collect();
        for review in reviews {
            let Some(user) = review.user else { continue };
            let verdict = match review.state.as_str() {
                "APPROVED" => ReviewState::Approved,
                "CHANGES_REQUESTED" => ReviewState::ChangesRequested,
                "COMMENTED" | "DISMISSED" => ReviewState::Commented,
                _ => continue, // PENDING: a draft review nobody else sees
            };
            let i = *at.entry(user.login.clone()).or_insert_with(|| {
                out.push(person(user.login, ReviewState::Commented));
                out.len() - 1
            });
            let current = &mut out[i].state;
            // A re-request outranks past reviews; a later comment does not undo a verdict.
            match (*current, review.state.as_str()) {
                (ReviewState::Requested, _) => {}
                (_, "DISMISSED") => *current = ReviewState::Commented,
                (ReviewState::Approved | ReviewState::ChangesRequested, "COMMENTED") => {}
                _ => *current = verdict,
            }
        }
        let owner = &self.repo.owner;
        out.extend(requested.teams.into_iter().map(|t| Reviewer {
            name: format!("{owner}/{}", t.slug),
            team: true,
            state: ReviewState::Requested,
        }));
        Ok(out)
    }

    fn merge(&self, pr: &Request, strategy: Strategy) -> Result<Merged, ForgeError> {
        let method = match strategy {
            Strategy::Merge => "merge",
            Strategy::Squash => "squash",
            Strategy::Rebase => "rebase",
        };
        let body = json!({"merge_method": method, "sha": pr.head_sha});
        let merged: MergedJson = self.call(
            Method::Put,
            &format!("/pulls/{}/merge", pr.number),
            Some(body),
        )?;
        Ok(Merged { sha: merged.sha })
    }

    fn strategies(&self) -> Result<Vec<Strategy>, ForgeError> {
        // GitHub only returns the allow_* fields to tokens that can push; absent reads as not allowed.
        let repo: RepoJson = self.call(Method::Get, "", None)?;
        Ok([
            (repo.allow_merge_commit, Strategy::Merge),
            (repo.allow_squash_merge, Strategy::Squash),
            (repo.allow_rebase_merge, Strategy::Rebase),
        ]
        .into_iter()
        .filter_map(|(allowed, s)| allowed.then_some(s))
        .collect())
    }

    fn request_word(&self) -> &'static str {
        "PR"
    }

    fn checks_word(&self) -> &'static str {
        "checks"
    }
}
