//! Accept (spec §6; ADR 0011, 0020): a plan draft becomes a branch, a worktree and
//! `.arch/sessions/<id>/`.
//!
//! ```text
//! cache: plan draft ──▶ branch arch/<id> from HEAD
//!                       worktree <parent>/<repo>-w<n>   (lowest free n)
//!                       .arch/sessions/<id>/plan.toml · thread.jsonl · session.toml
//!                       one commit: chore(arch): plan <name>
//! cache: draft deleted
//! ```
//!
//! The plan is shaped (one gate per dependency layer) when it reaches Accept without groups. The
//! thread starts with the planner's entries, then arch's own `accepted` line. Accept · delegate
//! leaves the session Running with the first group to run; without delegate it is a locked
//! `you` session (Yours).

use std::path::{Path, PathBuf};
use std::process::Command;

use arch_facts::{
    ArchDir, Cursor, Plan, Session, SessionId, SessionState, Store, ThreadAuthor, ThreadEntry,
    ThreadEvent,
};

use crate::machine::{Trigger, next};
use crate::{Error, shape};

/// The gate's test command when none is configured (#48 makes it a setting).
pub const DEFAULT_TEST_COMMAND: &str = "cargo test --workspace";

/// How Accept places and shapes the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptOptions {
    /// Where the worktree goes; the repository's parent directory when `None` (ADR 0011).
    pub worktree_parent: Option<PathBuf>,
    /// The gate's test command, for a plan that reaches Accept unshaped.
    pub test_command: String,
    /// The session's name; the plan's intention when `None`.
    pub name: Option<String>,
    /// Accept · delegate (Running) rather than a locked `you` session (Yours).
    pub delegate: bool,
}

impl Default for AcceptOptions {
    fn default() -> Self {
        AcceptOptions {
            worktree_parent: None,
            test_command: DEFAULT_TEST_COMMAND.into(),
            name: None,
            delegate: true,
        }
    }
}

/// Accept the draft `id` from `store` for the repository at `repo`. `planner_thread` is the
/// planner's conversation, the first part of the session's thread.
pub fn accept(
    repo: &Path,
    store: &Store,
    id: &SessionId,
    planner_thread: &[ThreadEntry],
    options: &AcceptOptions,
) -> Result<Session, Error> {
    let mut plan = store
        .plan_draft(id.as_str())?
        .ok_or_else(|| Error::NoDraft(id.clone()))?;
    if plan.groups.is_empty() {
        plan.groups = shape(&plan, &options.test_command)?;
    }
    let repo = repo.canonicalize().map_err(|source| Error::Io {
        path: repo.to_path_buf(),
        source,
    })?;
    let base = git(&repo, &["rev-parse", "HEAD"])?;
    let branch = format!("arch/{id}");
    if git(
        &repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok()
    {
        return Err(Error::Git(format!("branch {branch} already exists")));
    }
    let worktree = free_worktree(&repo, options.worktree_parent.as_deref())?;
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            &worktree.to_string_lossy(),
            &base,
        ],
    )?;

    let written = write_session(&worktree, &plan, planner_thread, options, &branch, &base);
    let session = match written {
        Ok(s) => s,
        Err(e) => {
            // Leave no half-accepted session behind: the draft is still in the cache.
            let _ = git(
                &repo,
                &["worktree", "remove", "--force", &worktree.to_string_lossy()],
            );
            let _ = git(&repo, &["branch", "-D", &branch]);
            return Err(e);
        }
    };
    store.plan_draft_delete(id.as_str())?;
    Ok(session)
}

fn write_session(
    worktree: &Path,
    plan: &Plan,
    planner_thread: &[ThreadEntry],
    options: &AcceptOptions,
    branch: &str,
    base: &str,
) -> Result<Session, Error> {
    let id = &plan.session;
    let name = options
        .name
        .clone()
        .unwrap_or_else(|| plan.intention.clone());
    let trigger = if options.delegate {
        Trigger::Delegate
    } else {
        Trigger::SavePlan
    };
    let state = next(SessionState::Planning, trigger)?;
    let session = Session {
        id: id.clone(),
        name: name.clone(),
        state,
        branch: Some(branch.to_string()),
        worktree: Some(worktree.to_path_buf()),
        base: Some(base.to_string()),
        driver: None,
        created: arch_facts::now(),
        cursor: Cursor {
            todo: match (state, plan.groups.first()) {
                (SessionState::Running, Some(g)) => g.elements.clone(),
                _ => vec![],
            },
            ..Cursor::default()
        },
        agents: vec![],
    };
    let dir = ArchDir::of_repo(worktree).session(id);
    dir.write_plan(plan)?;
    for entry in planner_thread {
        dir.append_thread(entry)?;
    }
    dir.append_thread(&ThreadEntry::new(
        ThreadAuthor::Arch,
        ThreadEvent::Text {
            text: format!(
                "accepted · branch {branch} · worktree {}",
                worktree.display()
            ),
        },
    ))?;
    dir.write_session(&session)?;

    let folder = format!(".arch/sessions/{id}");
    git(worktree, &["add", "--force", "--", &folder])?;
    git(
        worktree,
        &[
            "commit",
            "--quiet",
            "-m",
            &format!("chore(arch): plan {name}"),
            "-m",
            &format!("Arch-Session: {id}"),
            "--",
            &folder,
        ],
    )?;
    Ok(session)
}

/// `<parent>/<repo>-w<n>` with the lowest `n` from 1 that is free (ADR 0011).
fn free_worktree(repo: &Path, parent: Option<&Path>) -> Result<PathBuf, Error> {
    let parent = match parent {
        Some(p) => p.to_path_buf(),
        None => repo
            .parent()
            .ok_or_else(|| Error::Git(format!("{} has no parent directory", repo.display())))?
            .to_path_buf(),
    };
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    Ok((1..)
        .map(|n| parent.join(format!("{name}-w{n}")))
        .find(|p| !p.exists())
        .expect("some n is free"))
}

/// Run git in `dir`; its stdout, trimmed.
pub(crate) fn git(dir: &Path, args: &[&str]) -> Result<String, Error> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| Error::Git(format!("could not run git: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(Error::Git(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}
