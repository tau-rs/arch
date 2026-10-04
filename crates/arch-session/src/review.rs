//! Review and merge (spec §6; ADR 0003, 0016, 0017, 0018): open the request on the forge, then
//! merge it and archive the session.
//!
//! ```text
//! pr     Done ─ create_pr (pushes the branch) ─▶ InReview, committed in the session folder
//! merge  InReview ─ capture the folder ─▶ .arch/cache/archive/<id>.toml   (survives a failure)
//!                 ─ checks green? strategy allowed? ─ remove the folder, commit, push
//!                 ─ forge merge ─▶ .arch/cache/archive/<id>.merged        (the merge commit)
//!                 ─ fetch, note on the merge commit (refs/notes/arch), Merged ▶ Archived
//!                 ─ worktree and branch removed, the cache files deleted
//! ```
//!
//! The folder leaves the branch before the merge so main's tree never holds it (ADR 0003): the
//! merged head is one commit past the reviewed one. Every step can be re-run: a merge that the
//! forge refuses (checks still running on that commit) leaves the capture in the cache, and the
//! next `merge` goes on from it. A merge the forge calls not mergeable right after the push (it is
//! still computing mergeability) is retried a few times first ([`SETTLE`]).

use std::path::{Path, PathBuf};
use std::time::Duration;

use arch_facts::{
    ArchDir, Archive, Plan, RecordKind, Session, SessionId, SessionState, ThreadAuthor,
    ThreadEntry, ThreadEvent, Verdict,
};
use arch_forge::{
    CheckState, Forge, ForgeError, NewRequest, Request, RequestState, Strategy, summary,
};

use crate::engine::commit_folder;
use crate::machine::{Trigger, next};
use crate::{Error, git};

/// A request opened (or found open) for a session.
#[derive(Debug, Clone)]
pub struct Opened {
    /// The request.
    pub request: Request,
    /// The session record after it (InReview).
    pub session: Session,
    /// False when the request was already open.
    pub created: bool,
}

/// A session merged and archived.
#[derive(Debug, Clone)]
pub struct Merged {
    /// The request's number on the forge.
    pub number: u64,
    /// How it was merged.
    pub strategy: Strategy,
    /// The commit the merge produced; the note is on it.
    pub sha: String,
    /// The archive written to `refs/notes/arch`.
    pub archive: Archive,
    /// The worktree removed.
    pub worktree: Option<PathBuf>,
    /// The branch removed.
    pub branch: String,
}

/// Open the request for session `id` in `worktree`, into `base`: the forge pushes the branch
/// (ADR 0017) and the session goes Done → InReview. An open request for the branch is reused.
pub fn pr(worktree: &Path, id: &SessionId, forge: &dyn Forge, base: &str) -> Result<Opened, Error> {
    let dir = ArchDir::of_repo(worktree).session(id);
    let (Some(mut session), Some(plan)) = (dir.read_session()?, dir.read_plan()?) else {
        return Err(Error::NoSession(id.clone()));
    };
    if session.state != SessionState::InReview {
        next(session.state, Trigger::PrCreated)?;
    }
    let branch = branch_of(&session)?;
    let (request, created) = match forge.pr(&branch)? {
        Some(r) if r.state == RequestState::Open => (r, false),
        _ => {
            let new = NewRequest {
                head: branch.clone(),
                base: base.to_string(),
                title: session.name.clone(),
                body: description(&dir.read_records()?, &dir.read_thread()?, &plan, &session),
                draft: false,
            };
            (forge.create_pr(&new)?, true)
        }
    };
    if session.state != SessionState::InReview {
        session.state = next(session.state, Trigger::PrCreated)?;
        dir.write_session(&session)?;
    }
    if created {
        dir.append_thread(&ThreadEntry::new(
            ThreadAuthor::Arch,
            ThreadEvent::Text {
                text: format!(
                    "{} #{} · {} · in review",
                    forge.request_word(),
                    request.number,
                    request.url
                ),
            },
        ))?;
    }
    commit_folder(worktree, &session)?;
    Ok(Opened {
        request,
        session,
        created,
    })
}

/// The request's description, drafted from the plan, the gate lines and the records.
pub fn description(
    records: &[arch_facts::Record],
    thread: &[ThreadEntry],
    plan: &Plan,
    session: &Session,
) -> String {
    let verdict = |id: &arch_facts::ElementId| {
        records.iter().rev().find_map(|r| match &r.kind {
            RecordKind::JudgeVerdict {
                element,
                verdict,
                reason,
            } if element == id => Some(format!(
                "{} · {reason}",
                match verdict {
                    Verdict::Pass => "pass",
                    Verdict::Fail => "fail",
                }
            )),
            _ => None,
        })
    };
    let mut out = format!(
        "{}\n\narch session `{}` · {} element(s) in {} group(s).\n\n\
         | | element | site | files | judge |\n|---|---|---|---|---|\n",
        plan.intention,
        session.id,
        plan.elements.len(),
        plan.groups.len()
    );
    for e in &plan.elements {
        let files: Vec<String> = e
            .files
            .iter()
            .map(|f| format!("`{}`", f.display()))
            .collect();
        out.push_str(&format!(
            "| {} | {} | `{}` | {} | {} |\n",
            e.label,
            cell(&e.intention),
            e.site,
            files.join(" "),
            cell(&verdict(&e.id).unwrap_or_else(|| "—".into()))
        ));
    }
    let gates: Vec<&str> = thread
        .iter()
        .filter(|t| t.author == ThreadAuthor::Arch)
        .filter_map(|t| match &t.event {
            ThreadEvent::Text { text } if text.starts_with("gate · ") => Some(text.as_str()),
            _ => None,
        })
        .collect();
    if !gates.is_empty() {
        out.push_str("\n### Gates\n\n");
        for g in gates {
            out.push_str(&format!("- {g}\n"));
        }
    }
    let overrides: Vec<String> = records
        .iter()
        .filter_map(|r| match &r.kind {
            RecordKind::Override { what, reason, by } => {
                Some(format!("- {what} accepted as is by {by}: {reason}"))
            }
            _ => None,
        })
        .collect();
    if !overrides.is_empty() {
        out.push_str("\n### Overrides\n\n");
        out.push_str(&overrides.join("\n"));
        out.push('\n');
    }
    out.push_str(&format!(
        "\nThe plan, the thread and the records are in `.arch/sessions/{}/`; they leave the branch \
         at merge and are archived to `refs/notes/arch` (ADR 0003).\n\nArch-Session: {}\n",
        session.id, session.id
    ));
    out
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

/// Merge session `id`'s request with `strategy` (or the one strategy the repo allows), then
/// archive the session: the note on the merge commit in `repo`, the worktree and branch removed.
pub fn merge(
    repo: &Path,
    id: &SessionId,
    worktree: Option<&Path>,
    forge: &dyn Forge,
    strategy: Option<Strategy>,
) -> Result<Merged, Error> {
    let cache = repo.join(".arch/cache/archive");
    let pending = cache.join(format!("{id}.toml"));
    let merged_at = cache.join(format!("{id}.merged"));

    let mut archive = match read(&pending)? {
        Some(text) => Archive::from_toml(&text)?,
        None => {
            let worktree = worktree.ok_or_else(|| Error::NoSession(id.clone()))?;
            let dir = ArchDir::of_repo(worktree).session(id);
            let session = dir
                .read_session()?
                .ok_or_else(|| Error::NoSession(id.clone()))?;
            next(session.state, Trigger::Merged)?;
            let archive = Archive::of(&dir)?;
            write(&pending, &archive.to_toml()?)?;
            archive
        }
    };
    let mut session = archive
        .session()?
        .ok_or_else(|| Error::NoSession(id.clone()))?;
    let branch = branch_of(&session)?;
    let word = forge.request_word();

    let request = forge
        .pr(&branch)?
        .ok_or_else(|| Error::NoRequest(id.clone(), word))?;
    let (sha, strategy) = match (read(&merged_at)?, request.state) {
        (Some(done), _) => parse_merged(&done)?,
        (None, RequestState::Closed) => {
            return Err(Error::RequestClosed(format!("{word} #{}", request.number)));
        }
        (None, RequestState::Merged) => {
            return Err(Error::Git(format!(
                "{word} #{} was merged outside arch; arch cannot tell its merge commit",
                request.number
            )));
        }
        (None, RequestState::Open) => {
            match summary(&forge.checks(&request)?) {
                Some(CheckState::Failed) => {
                    return Err(Error::ChecksNotGreen {
                        what: forge.checks_word(),
                        state: "failed",
                    });
                }
                Some(CheckState::Pending) => {
                    return Err(Error::ChecksNotGreen {
                        what: forge.checks_word(),
                        state: "pending",
                    });
                }
                _ => {}
            }
            let strategy = choose(&forge.strategies()?, strategy)?;
            let worktree = worktree.ok_or_else(|| Error::NoSession(id.clone()))?;
            let head = strip(worktree, &session)?;
            forge.push(&branch)?;
            let request = Request {
                head_sha: head,
                ..request.clone()
            };
            let done = merge_settled(forge, &request, strategy, id)?;
            write(
                &merged_at,
                &format!("{} {}", done.sha, strategy_word(strategy)),
            )?;
            (done.sha, strategy)
        }
    };

    session.state = next(session.state, Trigger::Merged)?;
    session.state = next(session.state, Trigger::Archived)?;
    archive.set_session(&session)?;
    append_line(
        &mut archive,
        &format!(
            "merged {word} #{} ({}) as {sha} · archived",
            request.number,
            strategy_word(strategy)
        ),
    )?;
    git(repo, &["fetch", "--quiet", "origin"])?;
    if git(repo, &["cat-file", "-e", &format!("{sha}^{{commit}}")]).is_err() {
        return Err(Error::Git(format!(
            "the merge commit {sha} is not on origin yet; run `arch session merge {id}` again"
        )));
    }
    archive.write_note(repo, &sha)?;

    if let Some(w) = worktree.filter(|w| w.exists()) {
        git(
            repo,
            &["worktree", "remove", "--force", &w.to_string_lossy()],
        )?;
    }
    if git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok()
    {
        git(repo, &["branch", "-D", &branch])?;
    }
    for p in [&pending, &merged_at] {
        let _ = std::fs::remove_file(p);
    }
    Ok(Merged {
        number: request.number,
        strategy,
        sha,
        archive,
        worktree: worktree.map(Path::to_path_buf),
        branch,
    })
}

/// How long to wait before each retry of a merge the forge calls not mergeable. Right after a
/// push GitHub is still computing the request's mergeability and answers 405 (seen in #6's real
/// run, the push being the archive commit).
pub const SETTLE: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];

/// `Forge::merge`, retried over [`SETTLE`] while the forge answers 405 "not mergeable".
fn merge_settled(
    forge: &dyn Forge,
    request: &Request,
    strategy: Strategy,
    id: &SessionId,
) -> Result<arch_forge::Merged, Error> {
    let mut waits = SETTLE.iter();
    loop {
        match forge.merge(request, strategy) {
            Err(ForgeError::Rejected {
                status: 405,
                message,
            }) if message.contains("not mergeable") => match waits.next() {
                Some(wait) => std::thread::sleep(*wait),
                None => {
                    return Err(Error::NotMergeable(format!(
                        "{} #{} is not mergeable: the forge is still checking the last push, \
                             or it conflicts with its base; run `arch session merge {id}` again",
                        forge.request_word(),
                        request.number
                    )));
                }
            },
            other => return Ok(other?),
        }
    }
}

/// The strategy to merge with: the one asked for when the repo allows it, else the repo's only
/// one. Never chosen by arch (ADR 0016).
pub fn choose(allowed: &[Strategy], asked: Option<Strategy>) -> Result<Strategy, Error> {
    let list = || {
        allowed
            .iter()
            .map(|s| strategy_word(*s))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match (asked, allowed) {
        (Some(s), _) if allowed.contains(&s) => Ok(s),
        (Some(s), _) => Err(Error::Strategy(format!(
            "the repo does not allow {}; it allows {}",
            strategy_word(s),
            list()
        ))),
        (None, [only]) => Ok(*only),
        (None, []) => Err(Error::Strategy("the repo allows no merge strategy".into())),
        (None, _) => Err(Error::Strategy(format!(
            "the repo allows {}; name one with --strategy",
            list()
        ))),
    }
}

/// A strategy's word: `merge`, `squash`, `rebase`.
pub fn strategy_word(s: Strategy) -> &'static str {
    match s {
        Strategy::Merge => "merge",
        Strategy::Squash => "squash",
        Strategy::Rebase => "rebase",
    }
}

/// Remove the session folder from the branch (`chore(arch): archive <name>`), once; the head.
fn strip(worktree: &Path, session: &Session) -> Result<String, Error> {
    let folder = format!(".arch/sessions/{}", session.id);
    let tracked = git(worktree, &["ls-tree", "-d", "--name-only", "HEAD", &folder])?;
    if !tracked.is_empty() {
        git(worktree, &["rm", "-r", "--quiet", "--", &folder])?;
        git(
            worktree,
            &[
                "commit",
                "--quiet",
                "-m",
                &format!("chore(arch): archive {}", session.name),
                "-m",
                &format!("Arch-Session: {}", session.id),
                "--",
                &folder,
            ],
        )?;
    }
    git(worktree, &["rev-parse", "HEAD"])
}

fn append_line(archive: &mut Archive, text: &str) -> Result<(), Error> {
    let entry = ThreadEntry::new(ThreadAuthor::Arch, ThreadEvent::Text { text: text.into() });
    let line = serde_json::to_string(&entry).map_err(|e| Error::Plan(e.to_string()))?;
    if let Some(f) = archive
        .files
        .iter_mut()
        .find(|f| f.path == Path::new("thread.jsonl"))
    {
        if !f.content.is_empty() && !f.content.ends_with('\n') {
            f.content.push('\n');
        }
        f.content.push_str(&line);
        f.content.push('\n');
    }
    Ok(())
}

fn parse_merged(text: &str) -> Result<(String, Strategy), Error> {
    let mut parts = text.split_whitespace();
    let sha = parts.next().unwrap_or_default().to_string();
    let strategy = match parts.next() {
        Some("squash") => Strategy::Squash,
        Some("rebase") => Strategy::Rebase,
        _ => Strategy::Merge,
    };
    Ok((sha, strategy))
}

fn branch_of(session: &Session) -> Result<String, Error> {
    session
        .branch
        .clone()
        .ok_or_else(|| Error::Git(format!("session {} has no branch", session.id)))
}

fn read(path: &Path) -> Result<Option<String>, Error> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn write(path: &Path, text: &str) -> Result<(), Error> {
    let io = |source| Error::Io {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    std::fs::write(path, text).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arch_facts::{Cursor, Record};

    #[test]
    fn the_description_lists_elements_verdicts_gates_and_overrides() {
        let id = SessionId::new("3c9e1f0a");
        let mut plan = Plan::new(id.clone(), "add a refund flow");
        let e1 = plan
            .add_element("add Refund | to the domain", "src/domain/refund.rs")
            .id
            .clone();
        plan.elements[0].files = vec!["src/domain/refund.rs".into()];
        plan.add_element("RefundRepo port", "src/ports/refunds.rs");
        plan.groups = crate::shape(&plan, "cargo test").unwrap();
        let session = Session {
            id: id.clone(),
            name: "add a refund flow".into(),
            state: SessionState::Done,
            branch: Some("arch/3c9e1f0a".into()),
            worktree: None,
            base: None,
            driver: None,
            created: arch_facts::now(),
            cursor: Cursor::default(),
            agents: vec![],
        };
        let record = |seq, kind| Record {
            seq,
            at: arch_facts::now(),
            kind,
            witnesses: vec![],
        };
        let records = vec![
            record(
                1,
                RecordKind::JudgeVerdict {
                    element: e1.clone(),
                    verdict: Verdict::Fail,
                    reason: "not persisted".into(),
                },
            ),
            record(
                2,
                RecordKind::JudgeVerdict {
                    element: e1,
                    verdict: Verdict::Pass,
                    reason: "realized".into(),
                },
            ),
            record(
                3,
                RecordKind::Override {
                    what: "gate of group 1".into(),
                    reason: "flaky test".into(),
                    by: "Person".into(),
                },
            ),
        ];
        let thread = vec![
            ThreadEntry::new(
                ThreadAuthor::Arch,
                ThreadEvent::Text {
                    text: "gate · group 1 · cargo test ✓ · judge 2/2 pass".into(),
                },
            ),
            ThreadEntry::new(
                ThreadAuthor::Planner,
                ThreadEvent::Text {
                    text: "gate · not arch's".into(),
                },
            ),
        ];
        let body = description(&records, &thread, &plan, &session);
        assert!(body.starts_with(
            "add a refund flow\n\narch session `3c9e1f0a` · 2 element(s) in 1 group(s)."
        ));
        assert!(body.contains(
            "| E1 | add Refund \\| to the domain | `src/domain/refund.rs` | `src/domain/refund.rs` | pass · realized |"
        ), "{body}");
        assert!(
            body.contains("| E2 | RefundRepo port | `src/ports/refunds.rs` |  | — |"),
            "{body}"
        );
        assert!(body.contains("### Gates\n\n- gate · group 1 · cargo test ✓ · judge 2/2 pass\n\n"));
        assert!(!body.contains("not arch's"));
        assert!(body.contains("- gate of group 1 accepted as is by Person: flaky test"));
        assert!(body.ends_with("Arch-Session: 3c9e1f0a\n"));
    }

    #[test]
    fn the_strategy_is_the_repos_never_arch_s() {
        use Strategy::*;
        assert_eq!(choose(&[Squash], None).unwrap(), Squash);
        assert_eq!(choose(&[Merge, Squash], Some(Merge)).unwrap(), Merge);
        assert_eq!(
            choose(&[Merge, Squash, Rebase], None)
                .unwrap_err()
                .to_string(),
            "the repo allows merge, squash, rebase; name one with --strategy"
        );
        assert_eq!(
            choose(&[Squash], Some(Rebase)).unwrap_err().to_string(),
            "the repo does not allow rebase; it allows squash"
        );
        assert!(choose(&[], None).is_err());
    }
}
