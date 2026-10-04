//! `arch session …` (#48): one delegated session from the CLI, the methods milestone 6 serves
//! over JSON-RPC and MCP unchanged.
//!
//! | method | what | engine |
//! |---|---|---|
//! | [`session_new`] | the planner (or `plan.toml`), the draft to the cache; Accept and run with `delegate` | `planner`, `shape` |
//! | [`session_accept`] | branch, worktree, session folder; run with `delegate` | `accept`, `Engine::run` |
//! | [`session_run`] | run on from where the record stands (restart, ADR 0015) | `Engine::run` |
//! | [`session_status`] | the record, the plan, the gate lines, what it waits on | — |
//! | [`session_answer`] | answer the open ask | `Engine::answer` |
//! | [`session_decide`] | a gate door or a deviation typology | `decide_gate`, `decide_deviation` |
//! | [`session_pr`] | push and open the request | `review::pr` |
//! | [`session_merge`] | merge with the repo's strategy, archive, remove worktree and branch | `review::merge` |
//!
//! Every method takes a path inside the repository or one of its worktrees, and the session id.
//! A session is found through `git worktree list` (branch `arch/<id>`), then the merge cache,
//! then the plan drafts, then the notes. Between `new` and `accept` the planner's thread waits
//! in the cache next to the draft (`.arch/cache/drafts/<id>.thread.jsonl`, ADR 0020).

use std::path::{Path, PathBuf};
use std::process::Command;

use arch_driver::{ClaudeCode, Context, Driver, ReplayDriver};
use arch_facts::{
    ArchDir, Archive, ContentHash, NOTES_REF, Plan, Question, Session, SessionId, SessionState,
    Store, ThreadAuthor, ThreadEntry, ThreadEvent,
};
use arch_forge::{Forge, GitHub, Recorded, RepoRef};
use arch_session::{
    AcceptOptions, Door, Engine, EngineOptions, Typology, accept, pack, planner, review, shape,
};
use serde::Serialize;

pub use arch_forge::Strategy;
pub use arch_session::accept::DEFAULT_TEST_COMMAND;

use crate::{ArchProject, Error};

/// Which agent runtime runs the planner, the elements and the judge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverChoice {
    /// `claude -p` (ADR 0012).
    ClaudeCode,
    /// Recorded turns, `<dir>/*.jsonl` in name order, acted out through the tool layer
    /// (`ARCH_DRIVER=replay:<dir>`, tests).
    Replay(PathBuf),
}

/// Which forge the session's request goes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForgeChoice {
    /// GitHub, from `origin` (ADR 0018).
    GitHub,
    /// Recorded GitHub answers, `<dir>/recorded.json`; pushes still go to `origin`
    /// (`ARCH_FORGE=fake:<dir>`, tests).
    Fake(PathBuf),
}

/// How sessions run: flags over the environment for now; the Settings tab comes later
/// (ADR 0023).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionConfig {
    /// The gate's test command (`ARCH_TEST_COMMAND`).
    pub test_command: String,
    /// Where worktrees go (`ARCH_WORKTREE_PARENT`); the repository's parent when `None`.
    pub worktree_parent: Option<PathBuf>,
    /// The planner's and elements' model (`ARCH_MODEL`).
    pub model: Option<String>,
    /// The judge's model (`ARCH_JUDGE_MODEL`).
    pub judge_model: Option<String>,
    /// A cap on an element turn's agentic turns (`ARCH_MAX_TURNS`).
    pub max_turns: Option<u32>,
    /// The driver (`ARCH_DRIVER`: `claude-code` or `replay:<dir>`).
    pub driver: DriverChoice,
    /// The forge (`ARCH_FORGE`: `github` or `fake:<dir>`).
    pub forge: ForgeChoice,
    /// The `arch` binary the hooks and the MCP server run; the running one when `None`.
    pub exe: Option<PathBuf>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            test_command: DEFAULT_TEST_COMMAND.into(),
            worktree_parent: None,
            model: None,
            judge_model: None,
            max_turns: None,
            driver: DriverChoice::ClaudeCode,
            forge: ForgeChoice::GitHub,
            exe: None,
        }
    }
}

impl SessionConfig {
    /// The defaults, overridden by `ARCH_*` variables read through `var`.
    pub fn from_env(var: impl Fn(&str) -> Option<String>) -> Result<Self, Error> {
        let var = |k: &str| var(k).filter(|v| !v.is_empty());
        let mut c = SessionConfig::default();
        if let Some(t) = var("ARCH_TEST_COMMAND") {
            c.test_command = t;
        }
        c.worktree_parent = var("ARCH_WORKTREE_PARENT").map(PathBuf::from);
        c.model = var("ARCH_MODEL");
        c.judge_model = var("ARCH_JUDGE_MODEL");
        if let Some(n) = var("ARCH_MAX_TURNS") {
            c.max_turns = Some(
                n.parse()
                    .map_err(|_| Error::Config(format!("ARCH_MAX_TURNS={n}: not a number")))?,
            );
        }
        if let Some(d) = var("ARCH_DRIVER") {
            c.driver = match d.split_once(':') {
                None if d == "claude-code" => DriverChoice::ClaudeCode,
                Some(("replay", dir)) => DriverChoice::Replay(dir.into()),
                _ => {
                    return Err(Error::Config(format!(
                        "ARCH_DRIVER={d}: expected claude-code or replay:<dir>"
                    )));
                }
            };
        }
        if let Some(f) = var("ARCH_FORGE") {
            c.forge = match f.split_once(':') {
                None if f == "github" => ForgeChoice::GitHub,
                Some(("fake", dir)) => ForgeChoice::Fake(dir.into()),
                _ => {
                    return Err(Error::Config(format!(
                        "ARCH_FORGE={f}: expected github or fake:<dir>"
                    )));
                }
            };
        }
        Ok(c)
    }

    fn driver(&self) -> Result<Box<dyn Driver>, Error> {
        Ok(match &self.driver {
            DriverChoice::ClaudeCode => Box::new(ClaudeCode::new()),
            DriverChoice::Replay(dir) => Box::new(ReplayDriver::from_dir(dir)?.acting()),
        })
    }

    fn forge(&self, repo: &Path) -> Result<Box<dyn Forge>, Error> {
        Ok(match &self.forge {
            ForgeChoice::GitHub => Box::new(GitHub::open(repo)?),
            ForgeChoice::Fake(dir) => Box::new(GitHub::new(
                RepoRef::new("fake", "repo"),
                repo.to_path_buf(),
                Recorded::from_dir(dir)?,
            )),
        })
    }

    fn engine(&self) -> EngineOptions {
        EngineOptions {
            exe: self.exe.clone(),
            model: self.model.clone(),
            judge_model: self.judge_model.clone(),
            max_turns: self.max_turns,
        }
    }
}

/// A session as it stands: what `arch session status` prints, and what every method that
/// moves a session returns.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionReport {
    /// The id.
    pub id: String,
    /// The name (the intention unless named).
    pub name: String,
    /// Where it stands.
    pub state: SessionState,
    /// The branch, from Accept until archived.
    pub branch: Option<String>,
    /// The worktree, from Accept until archived.
    pub worktree: Option<PathBuf>,
    /// The plan.
    pub plan: Plan,
    /// The group the scheduler stands on (0-based).
    pub group: u32,
    /// The gate lines of the thread, in order (`gate · group 1 · … · judge 4/4 pass`).
    pub gates: Vec<String>,
    /// The open questions, in Asks and GateFailed.
    pub questions: Vec<Question>,
    /// The paths whose writes were denied, in Deviation.
    pub denied: Vec<PathBuf>,
    /// The merge commit the archive note is on, once archived.
    pub archived_on: Option<String>,
}

impl SessionReport {
    fn of(session: &Session, plan: Plan, thread: &[ThreadEntry]) -> Self {
        let gates = thread
            .iter()
            .filter(|t| t.author == ThreadAuthor::Arch)
            .filter_map(|t| match &t.event {
                ThreadEvent::Text { text } if text.starts_with("gate · ") => Some(text.clone()),
                _ => None,
            })
            .collect();
        let questions = match session.state {
            SessionState::Asks | SessionState::GateFailed => thread
                .iter()
                .rev()
                .find_map(|t| match &t.event {
                    ThreadEvent::Ask { questions } => Some(questions.clone()),
                    _ => None,
                })
                .unwrap_or_default(),
            _ => vec![],
        };
        let denied = match session.state {
            SessionState::Deviation => session
                .cursor
                .waiting
                .as_ref()
                .map(|w| w.denied.clone())
                .unwrap_or_default(),
            _ => vec![],
        };
        SessionReport {
            id: session.id.to_string(),
            name: session.name.clone(),
            state: session.state,
            branch: session.branch.clone(),
            worktree: session.worktree.clone(),
            plan,
            group: session.cursor.group,
            gates,
            questions,
            denied,
            archived_on: None,
        }
    }

    fn draft(plan: Plan) -> Self {
        SessionReport {
            id: plan.session.to_string(),
            name: plan.intention.clone(),
            state: SessionState::Planning,
            branch: None,
            worktree: None,
            plan,
            group: 0,
            gates: vec![],
            questions: vec![],
            denied: vec![],
            archived_on: None,
        }
    }
}

/// A gate door or a deviation typology (spec §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// One more fix round, with a hint for the agents.
    OneMore {
        /// The hint.
        hint: Option<String>,
    },
    /// Accept the gate as is; an Override record keeps the reason (ADR 0013).
    AcceptAsIs {
        /// Why.
        reason: String,
        /// Who; git's `user.name` when `None`.
        by: Option<String>,
    },
    /// The deviating agent goes back to its element's files.
    BackOnPlan,
    /// The denied paths join the element's files.
    UpdatePlan,
    /// The denied change is not part of this session.
    NotThisChange,
}

/// The request a session's review goes through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrReport {
    /// The forge's word for it (`PR`).
    pub word: String,
    /// Its number.
    pub number: u64,
    /// Its page.
    pub url: String,
    /// False when it was already open.
    pub created: bool,
}

/// A session merged and archived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MergeReport {
    /// The forge's word for the request (`PR`).
    pub word: String,
    /// The request's number.
    pub number: u64,
    /// `merge`, `squash` or `rebase`.
    pub strategy: String,
    /// The merge commit; the archive note is on it.
    pub sha: String,
    /// The files the archive holds.
    pub files: Vec<PathBuf>,
    /// The worktree removed.
    pub worktree: Option<PathBuf>,
    /// The branch removed.
    pub branch: String,
}

/// Plan `intention` (with the planner, or from `plan_file`) and keep the draft in the cache;
/// with `delegate`, Accept it and run.
pub fn session_new(
    path: &Path,
    intention: &str,
    plan_file: Option<&Path>,
    delegate: bool,
    config: &SessionConfig,
) -> Result<SessionReport, Error> {
    let repo = main_repo(path)?;
    let id = SessionId::new(
        ContentHash::of_str(&format!(
            "{intention}\n{}\n{}",
            arch_facts::now(),
            std::process::id()
        ))
        .0[..8]
            .to_string(),
    );
    let mut thread = vec![ThreadEntry::new(
        ThreadAuthor::You,
        ThreadEvent::Text {
            text: intention.into(),
        },
    )];
    let mut driver = None;
    let elements = match plan_file {
        Some(file) => {
            thread.push(ThreadEntry::new(
                ThreadAuthor::Arch,
                ThreadEvent::Text {
                    text: format!("plan read from {}", file.display()),
                },
            ));
            planner::read_plan_file(file)?
        }
        None => {
            let d = driver.insert(config.driver()?);
            let context = Context {
                pack: planner_pack(&repo, &id, intention)?,
                ..Context::default()
            };
            let planning = planner::Planning {
                repo: &repo,
                intention,
                model: config.model.clone(),
            };
            let (elements, entries) = planner::run(d.as_mut(), &planning, &context)?;
            thread.extend(entries);
            elements
        }
    };
    let mut plan = planner::draft(&id, intention, &elements)?;
    plan.groups = shape(&plan, &config.test_command)?;
    let store = Store::open(&repo.join(".arch"))?;
    store.plan_draft_put(&plan)?;
    write_draft_thread(&repo, &id, &thread)?;
    if !delegate {
        return Ok(SessionReport::draft(plan));
    }
    let mut driver = match driver {
        Some(d) => d,
        None => config.driver()?,
    };
    accept_and_run(&repo, &id, config, Some(driver.as_mut()))
}

/// Accept draft `id`: branch, worktree, session folder; with `delegate`, run until the session
/// needs a person or is done.
pub fn session_accept(
    path: &Path,
    id: &str,
    delegate: bool,
    config: &SessionConfig,
) -> Result<SessionReport, Error> {
    let repo = main_repo(path)?;
    let id = SessionId::new(id);
    if !delegate {
        return accept_and_run(&repo, &id, config, None);
    }
    let mut driver = config.driver()?;
    accept_and_run(&repo, &id, config, Some(driver.as_mut()))
}

/// Run session `id` on from where its record stands (after a crash or a restart, ADR 0015).
pub fn session_run(path: &Path, id: &str, config: &SessionConfig) -> Result<SessionReport, Error> {
    with_engine(path, id, config, |engine| engine.run())
}

/// The session as it stands, wherever it is: a draft, a worktree, a merge in progress, a note.
pub fn session_status(path: &Path, id: &str) -> Result<SessionReport, Error> {
    let repo = main_repo(path)?;
    let sid = SessionId::new(id);
    if let Some(worktree) = worktree_of(&repo, &sid)? {
        let dir = ArchDir::of_repo(&worktree).session(&sid);
        if let (Some(session), Some(plan)) = (dir.read_session()?, dir.read_plan()?) {
            return Ok(SessionReport::of(&session, plan, &dir.read_thread()?));
        }
    }
    let pending = repo.join(format!(".arch/cache/archive/{id}.toml"));
    if let Ok(text) = std::fs::read_to_string(&pending) {
        return report_of_archive(&Archive::from_toml(&text)?, None);
    }
    if repo.join(".arch/cache").is_dir()
        && let Some(plan) = Store::open(&repo.join(".arch"))?.plan_draft(id)?
    {
        return Ok(SessionReport::draft(plan));
    }
    for (_, commit) in notes(&repo)? {
        if let Some(archive) = Archive::read_note(&repo, &commit)?
            && archive.session == sid
        {
            return report_of_archive(&archive, Some(commit));
        }
    }
    Err(Error::NoSession(id.into()))
}

/// Answer the open ask of session `id`, and run on.
pub fn session_answer(
    path: &Path,
    id: &str,
    answers: Vec<String>,
    config: &SessionConfig,
) -> Result<SessionReport, Error> {
    with_engine(path, id, config, |engine| engine.answer(answers))
}

/// Settle what session `id` waits on with a door or a typology, and run on.
pub fn session_decide(
    path: &Path,
    id: &str,
    decision: Decision,
    config: &SessionConfig,
) -> Result<SessionReport, Error> {
    let repo = main_repo(path)?;
    let decision = match decision {
        Decision::AcceptAsIs { reason, by: None } => Decision::AcceptAsIs {
            reason,
            by: Some(git(&repo, &["config", "user.name"]).unwrap_or_else(|_| "you".into())),
        },
        d => d,
    };
    with_engine(path, id, config, |engine| match decision {
        Decision::OneMore { hint } => engine.decide_gate(Door::OneMore { hint }),
        Decision::AcceptAsIs { reason, by } => engine.decide_gate(Door::AcceptAsIs {
            reason,
            by: by.unwrap_or_default(),
        }),
        Decision::BackOnPlan => engine.decide_deviation(Typology::BackOnPlan),
        Decision::UpdatePlan => engine.decide_deviation(Typology::UpdatePlan),
        Decision::NotThisChange => engine.decide_deviation(Typology::NotThisChange),
    })
}

/// Push session `id`'s branch and open its request into `base` (the repository's current branch
/// when `None`); the session goes InReview.
pub fn session_pr(
    path: &Path,
    id: &str,
    base: Option<&str>,
    config: &SessionConfig,
) -> Result<PrReport, Error> {
    let repo = main_repo(path)?;
    let sid = SessionId::new(id);
    let worktree = worktree_of(&repo, &sid)?.ok_or_else(|| Error::NoSession(id.into()))?;
    let base = match base {
        Some(b) => b.to_string(),
        None => git(&repo, &["symbolic-ref", "--short", "HEAD"])?,
    };
    let forge = config.forge(&repo)?;
    let opened = review::pr(&worktree, &sid, forge.as_ref(), &base)?;
    Ok(PrReport {
        word: forge.request_word().into(),
        number: opened.request.number,
        url: opened.request.url,
        created: opened.created,
    })
}

/// Merge session `id`'s request with `strategy` (needed when the repo allows several), archive
/// the session to `refs/notes/arch` and remove its worktree and branch.
pub fn session_merge(
    path: &Path,
    id: &str,
    strategy: Option<Strategy>,
    config: &SessionConfig,
) -> Result<MergeReport, Error> {
    let repo = main_repo(path)?;
    let sid = SessionId::new(id);
    let worktree = worktree_of(&repo, &sid)?;
    let forge = config.forge(&repo)?;
    let merged = review::merge(&repo, &sid, worktree.as_deref(), forge.as_ref(), strategy)?;
    Ok(MergeReport {
        word: forge.request_word().into(),
        number: merged.number,
        strategy: review::strategy_word(merged.strategy).into(),
        sha: merged.sha,
        files: merged
            .archive
            .files
            .iter()
            .map(|f| f.path.clone())
            .collect(),
        worktree: merged.worktree,
        branch: merged.branch,
    })
}

/// Accept, then run with `driver` when there is one (Accept · delegate); without, the session
/// is a locked `you` session.
fn accept_and_run(
    repo: &Path,
    id: &SessionId,
    config: &SessionConfig,
    driver: Option<&mut dyn Driver>,
) -> Result<SessionReport, Error> {
    let store = Store::open(&repo.join(".arch"))?;
    let thread_file = draft_thread_path(repo, id);
    let thread = read_draft_thread(&thread_file)?;
    let session = accept(
        repo,
        &store,
        id,
        &thread,
        &AcceptOptions {
            worktree_parent: config.worktree_parent.clone(),
            test_command: config.test_command.clone(),
            name: None,
            delegate: driver.is_some(),
        },
    )?;
    let _ = std::fs::remove_file(&thread_file);
    let worktree = session
        .worktree
        .clone()
        .ok_or_else(|| Error::NoSession(id.to_string()))?;
    if let Some(driver) = driver {
        Engine::open(&worktree, id, driver, &ArchProject, config.engine())?.run()?;
    }
    let dir = ArchDir::of_repo(&worktree).session(id);
    let (Some(session), Some(plan)) = (dir.read_session()?, dir.read_plan()?) else {
        return Err(Error::NoSession(id.to_string()));
    };
    Ok(SessionReport::of(&session, plan, &dir.read_thread()?))
}

fn with_engine(
    path: &Path,
    id: &str,
    config: &SessionConfig,
    step: impl FnOnce(&mut Engine<'_>) -> Result<SessionState, arch_session::Error>,
) -> Result<SessionReport, Error> {
    let repo = main_repo(path)?;
    let sid = SessionId::new(id);
    let worktree = worktree_of(&repo, &sid)?.ok_or_else(|| Error::NoSession(id.into()))?;
    let mut driver = config.driver()?;
    let mut engine = Engine::open(
        &worktree,
        &sid,
        driver.as_mut(),
        &ArchProject,
        config.engine(),
    )?;
    step(&mut engine)?;
    let (session, plan) = (engine.session().clone(), engine.plan().clone());
    drop(engine);
    let dir = ArchDir::of_repo(&worktree).session(&sid);
    Ok(SessionReport::of(&session, plan, &dir.read_thread()?))
}

/// The planner's context pack: the repository's facts and `.arch/`, with an empty plan.
fn planner_pack(repo: &Path, id: &SessionId, intention: &str) -> Result<Option<PathBuf>, Error> {
    let Ok(facts) = arch_session::Project::facts(&ArchProject, repo) else {
        return Ok(None);
    };
    let text = pack::build(
        &facts,
        &ArchDir::of_repo(repo),
        &Plan::new(id.clone(), intention),
        None,
    )?;
    let dir = repo.join(".arch/cache/driver/planner");
    std::fs::create_dir_all(&dir).map_err(|source| Error::Io {
        path: dir.clone(),
        source,
    })?;
    let path = dir.join("pack.md");
    std::fs::write(&path, text).map_err(|source| Error::Io {
        path: path.clone(),
        source,
    })?;
    Ok(Some(path))
}

fn report_of_archive(archive: &Archive, on: Option<String>) -> Result<SessionReport, Error> {
    let file = |name: &str| {
        archive
            .files
            .iter()
            .find(|f| f.path == Path::new(name))
            .map(|f| f.content.clone())
    };
    let session = archive
        .session()?
        .ok_or_else(|| Error::NoSession(archive.session.to_string()))?;
    let plan: Plan = toml::from_str(&file("plan.toml").unwrap_or_default())
        .map_err(|e| Error::Config(format!("archived plan.toml: {}", e.message())))?;
    let thread: Vec<ThreadEntry> = file("thread.jsonl")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let mut report = SessionReport::of(&session, plan, &thread);
    report.archived_on = on;
    Ok(report)
}

/// The main repository of `path`, which may be one of its worktrees.
pub fn main_repo(path: &Path) -> Result<PathBuf, Error> {
    let common = git(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common = PathBuf::from(common);
    Ok(match common.file_name() {
        Some(n) if n == ".git" => common.parent().unwrap_or(&common).to_path_buf(),
        _ => common,
    })
}

/// The worktree checked out on `arch/<id>`, if any.
fn worktree_of(repo: &Path, id: &SessionId) -> Result<Option<PathBuf>, Error> {
    let list = git(repo, &["worktree", "list", "--porcelain"])?;
    let wanted = format!("branch refs/heads/arch/{id}");
    Ok(list.split("\n\n").find_map(|block| {
        let mut lines = block.lines();
        let path = lines.next()?.strip_prefix("worktree ")?;
        lines.any(|l| l == wanted).then(|| PathBuf::from(path))
    }))
}

/// `(note blob, annotated commit)` for every note on `refs/notes/arch`.
fn notes(repo: &Path) -> Result<Vec<(String, String)>, Error> {
    match git(repo, &["notes", "--ref", NOTES_REF, "list"]) {
        Ok(list) => Ok(list
            .lines()
            .filter_map(|l| l.split_once(' '))
            .map(|(n, c)| (n.to_string(), c.to_string()))
            .collect()),
        Err(_) => Ok(vec![]),
    }
}

fn draft_thread_path(repo: &Path, id: &SessionId) -> PathBuf {
    repo.join(format!(".arch/cache/drafts/{id}.thread.jsonl"))
}

fn write_draft_thread(repo: &Path, id: &SessionId, thread: &[ThreadEntry]) -> Result<(), Error> {
    let path = draft_thread_path(repo, id);
    let io = |source| Error::Io {
        path: path.clone(),
        source,
    };
    std::fs::create_dir_all(path.parent().expect("has a parent")).map_err(io)?;
    let mut text = String::new();
    for entry in thread {
        text.push_str(&serde_json::to_string(entry).expect("a thread entry serializes"));
        text.push('\n');
    }
    std::fs::write(&path, text).map_err(io)
}

fn read_draft_thread(path: &Path) -> Result<Vec<ThreadEntry>, Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                serde_json::from_str(l)
                    .map_err(|e| Error::Config(format!("{}: {e}", path.display())))
            })
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<String, Error> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_reads_the_environment_and_refuses_unknown_selectors() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(
            SessionConfig::from_env(env(&[])).unwrap(),
            SessionConfig::default()
        );
        let c = SessionConfig::from_env(env(&[
            ("ARCH_TEST_COMMAND", "true"),
            ("ARCH_DRIVER", "replay:/r"),
            ("ARCH_FORGE", "fake:/f"),
            ("ARCH_MAX_TURNS", "7"),
        ]))
        .unwrap();
        assert_eq!(c.test_command, "true");
        assert_eq!(c.driver, DriverChoice::Replay("/r".into()));
        assert_eq!(c.forge, ForgeChoice::Fake("/f".into()));
        assert_eq!(c.max_turns, Some(7));
        assert!(SessionConfig::from_env(env(&[("ARCH_DRIVER", "codex")])).is_err());
        assert!(SessionConfig::from_env(env(&[("ARCH_FORGE", "gitlab")])).is_err());
        assert!(SessionConfig::from_env(env(&[("ARCH_MAX_TURNS", "x")])).is_err());
    }
}
