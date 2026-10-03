//! The scheduler (spec §6, §11; ADR 0015): one group at a time, its elements one after another,
//! one driver session per element; the gate when a group is done; fix rounds, then the four-door
//! question.
//!
//! Every call reads the session record, runs until something needs a person or the plan is done,
//! and writes the record after every step, so the next call (another process: the CLI, #48)
//! picks up where this one stopped.
//!
//! ```text
//! run()  Running ─ todo[0] ─▶ element turn ─┬─ ask tool ─────────▶ Asks       ◀─ answer()
//!                                           ├─ scope denial ─────▶ Deviation  ◀─ decide_deviation()
//!                                           └─ done: next element
//!        Running, todo empty ─▶ Gate: commands · arch check · judge
//!                                 ├─ pass ─▶ next group, or Done
//!                                 ├─ fail, rounds left ─▶ Running (failing elements resumed)
//!                                 └─ fail, budget spent ─▶ GateFailed ◀─ decide_gate()
//! ```
//!
//! The session folder is committed (`chore(arch): <name> · <state>`) whenever `run` stops.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use arch_driver::tool_layer::config::{ToolLayerArgs, write_context};
use arch_driver::tool_layer::mcp;
use arch_driver::{Context, Driver, Task, TurnEvent};
use arch_facts::{
    ArchDir, DriverPointer, Element, ElementId, ElementState, Plan, Question, RecordKind, Session,
    SessionDir, SessionId, SessionState, ThreadAuthor, ThreadEntry, ThreadEvent, Verdict, Waiting,
};
use serde_json::Value;

use crate::denial::{Cause, cause};
use crate::gate::{self, tail};
use crate::judge::{self, Judging};
use crate::machine::{Trigger, next};
use crate::{Error, Project, git, pack};

/// The element prompt's template.
pub const ELEMENT_PROMPT: &str = include_str!("../prompts/element.md");

/// The built-in tools an element's agent has (`--tools`); the hooks guard the writes and limit
/// `Bash` to cargo and read-only git.
pub const ELEMENT_TOOLS: &[&str] = &["Read", "Edit", "Write", "Glob", "Grep", "Bash"];

/// The MCP tool whose call means the agent asked and ended its turn.
const ASK: &str = "mcp__arch__ask";

/// The four doors when the fix-round budget is spent (spec §6).
pub const FOUR_DOORS: [&str; 4] = [
    "one more round, with a hint",
    "take over",
    "accept as is, with a recorded reason",
    "re-plan",
];

/// How the engine runs its agents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineOptions {
    /// The `arch` binary the hooks and the MCP server run; the running binary when `None`.
    pub exe: Option<PathBuf>,
    /// The elements' model, when not the CLI's default.
    pub model: Option<String>,
    /// The judge's model, when not the CLI's default.
    pub judge_model: Option<String>,
    /// A cap on an element turn's agentic turns.
    pub max_turns: Option<u32>,
}

/// The answer to the four-door question (spec §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Door {
    /// One more fix round, with an optional hint for the agents.
    OneMore {
        /// What the person tells the failing elements' agents.
        hint: Option<String>,
    },
    /// Take over an element by hand (#49).
    TakeOver,
    /// Accept the gate as is; an Override record keeps the reason (ADR 0013).
    AcceptAsIs {
        /// Why.
        reason: String,
        /// Who.
        by: String,
    },
    /// Re-plan (#49).
    Replan,
}

/// The answer to a deviation (spec §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Typology {
    /// The agent goes back to the element's files.
    BackOnPlan,
    /// The denied paths join the element's files, and the agent goes ahead.
    UpdatePlan,
    /// The change is not part of this session; the agent leaves it.
    NotThisChange,
}

/// What an element turn ended with.
struct Outcome {
    asked: bool,
    denied: Vec<PathBuf>,
}

/// One session, in its worktree, driven by `driver`.
pub struct Engine<'a> {
    worktree: PathBuf,
    dir: SessionDir,
    session: Session,
    plan: Plan,
    driver: &'a mut dyn Driver,
    project: &'a dyn Project,
    options: EngineOptions,
    hint: Option<String>,
}

impl<'a> Engine<'a> {
    /// Open session `id` in `worktree` from its `session.toml` and `plan.toml`.
    pub fn open(
        worktree: &Path,
        id: &SessionId,
        driver: &'a mut dyn Driver,
        project: &'a dyn Project,
        options: EngineOptions,
    ) -> Result<Self, Error> {
        let worktree = worktree.canonicalize().map_err(|source| Error::Io {
            path: worktree.to_path_buf(),
            source,
        })?;
        let dir = ArchDir::of_repo(&worktree).session(id);
        let (Some(session), Some(plan)) = (dir.read_session()?, dir.read_plan()?) else {
            return Err(Error::NoSession(id.clone()));
        };
        Ok(Engine {
            worktree,
            dir,
            session,
            plan,
            driver,
            project,
            options,
            hint: None,
        })
    }

    /// The session record as it stands.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The plan as it stands.
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// Run until the session needs a person (Asks, Deviation, GateFailed) or is Done; any other
    /// state returns at once. The session folder is committed before returning.
    pub fn run(&mut self) -> Result<SessionState, Error> {
        loop {
            match self.session.state {
                SessionState::Running => match self.session.cursor.todo.first().cloned() {
                    Some(element) => {
                        let outcome = self.turn(&element, None)?;
                        self.after_turn(&element, outcome)?;
                    }
                    None => self.go(Trigger::GroupDone)?,
                },
                SessionState::Gate => self.gate()?,
                _ => break,
            }
        }
        self.commit_folder()?;
        Ok(self.session.state)
    }

    /// Answer the open ask: the answers go in the thread, the asking element's driver session is
    /// resumed with them, and the run goes on.
    pub fn answer(&mut self, answers: Vec<String>) -> Result<SessionState, Error> {
        next(self.session.state, Trigger::Answer)?;
        let waiting = self.waiting()?;
        self.dir.append_thread(&ThreadEntry::new(
            ThreadAuthor::You,
            ThreadEvent::Answer {
                answers: answers.clone(),
            },
        ))?;
        self.session.cursor.waiting = None;
        self.go(Trigger::Answer)?;
        let listed: Vec<String> = answers
            .iter()
            .enumerate()
            .map(|(i, a)| format!("{}. {a}", i + 1))
            .collect();
        let message = format!(
            "The person answered your questions, in order:\n{}\n\nContinue with the element.",
            listed.join("\n")
        );
        let outcome = self.turn(&waiting.element, Some(message))?;
        self.after_turn(&waiting.element, outcome)?;
        self.run()
    }

    /// Settle the open deviation with a typology, resume the element, and run on.
    pub fn decide_deviation(&mut self, typology: Typology) -> Result<SessionState, Error> {
        next(self.session.state, Trigger::Typology)?;
        let waiting = self.waiting()?;
        let element = self.element(&waiting.element)?;
        let paths = list(&waiting.denied);
        let (label, message) = match typology {
            Typology::BackOnPlan => (
                "back on the plan",
                format!(
                    "Your write to {paths} was denied: it is outside this element. The person \
                     says: back on the plan. Realize the element within its files ({}).",
                    list(&element.files)
                ),
            ),
            Typology::UpdatePlan => {
                let e = self.element_mut(&waiting.element)?;
                for p in &waiting.denied {
                    if !e.files.contains(p) {
                        e.files.push(p.clone());
                    }
                }
                self.dir.write_plan(&self.plan)?;
                (
                    "update the plan",
                    format!(
                        "The person added {paths} to this element's files. Go ahead with that \
                         change, then finish the element."
                    ),
                )
            }
            Typology::NotThisChange => (
                "not this change",
                format!(
                    "Your write to {paths} was denied, and the person says it is not part of \
                     this change. Leave it and finish the element within its files ({}).",
                    list(&element.files)
                ),
            ),
        };
        self.dir.append_thread(&ThreadEntry::new(
            ThreadAuthor::You,
            ThreadEvent::Answer {
                answers: vec![label.into()],
            },
        ))?;
        self.session.cursor.waiting = None;
        self.go(Trigger::Typology)?;
        let outcome = self.turn(&waiting.element, Some(message))?;
        self.after_turn(&waiting.element, outcome)?;
        self.run()
    }

    /// Answer the four-door question, then run on.
    pub fn decide_gate(&mut self, door: Door) -> Result<SessionState, Error> {
        let last = self.session.cursor.group as usize + 1 >= self.plan.groups.len();
        let trigger = match &door {
            Door::OneMore { .. } => Trigger::OneMore,
            Door::TakeOver => {
                return Err(Error::NotYet {
                    what: "take over",
                    issue: "#49",
                });
            }
            Door::Replan => Trigger::Replan,
            Door::AcceptAsIs { .. } if last => Trigger::AcceptAsIsLast,
            Door::AcceptAsIs { .. } => Trigger::AcceptAsIs,
        };
        next(self.session.state, trigger)?;
        let group = self.group()?.name.clone();
        match door {
            Door::OneMore { hint } => {
                let mut answers = vec![FOUR_DOORS[0].to_string()];
                answers.extend(hint.clone());
                self.answer_line(answers)?;
                self.session.cursor.extra_rounds += 1;
                self.session.cursor.fix_round += 1;
                self.hint = hint;
            }
            Door::AcceptAsIs { reason, by } => {
                self.answer_line(vec![FOUR_DOORS[2].to_string(), reason.clone()])?;
                self.dir.write_record(
                    RecordKind::Override {
                        what: format!("gate of {group}"),
                        reason,
                        by,
                    },
                    vec![],
                )?;
                if !last {
                    self.next_group();
                }
            }
            Door::Replan => {
                return Err(Error::NotYet {
                    what: "re-plan",
                    issue: "#49",
                });
            }
            Door::TakeOver => unreachable!("returned above"),
        }
        self.go(trigger)?;
        self.run()
    }

    /// One turn of `id`'s agent: a fresh driver session the first time, a resume after that.
    fn turn(&mut self, id: &ElementId, message: Option<String>) -> Result<Outcome, Error> {
        let element = self.element(id)?;
        self.element_mut(id)?.state = ElementState::Running;
        self.dir.write_plan(&self.plan)?;

        let pack = self.pack(Some(id), id.as_str())?;
        let exe = match &self.options.exe {
            Some(exe) => exe.clone(),
            None => std::env::current_exe().map_err(|source| Error::Io {
                path: PathBuf::from("<current exe>"),
                source,
            })?,
        };
        let args = ToolLayerArgs {
            exe,
            worktree: self.worktree.clone(),
            session: self.session.id.clone(),
            element: id.clone(),
        };
        let context = write_context(&args, pack)?;
        let previous = self.session.agent(id).map(|p| p.session_id.clone());
        let prompt = match (&previous, message) {
            (_, Some(m)) => m,
            (Some(_), None) if self.session.cursor.fix_round > 0 => self.fix_prompt(&element)?,
            (Some(_), None) => "Continue with the element where you left off.".to_string(),
            (None, None) => self.element_prompt(&element),
        };
        let mut allowed: Vec<String> = mcp::TOOLS.iter().map(|t| t.to_string()).collect();
        allowed.extend(["Read", "Glob", "Grep", "Edit", "Write"].map(String::from));
        let task = Task {
            prompt,
            cwd: self.worktree.clone(),
            tools: ELEMENT_TOOLS.iter().map(|t| t.to_string()).collect(),
            allowed_tools: allowed,
            output_schema: None,
            model: self.options.model.clone(),
            max_turns: self.options.max_turns,
        };

        let records_before = self.dir.read_records()?.len();
        let turn = match &previous {
            Some(sid) => self.driver.resume(sid, &task, &context)?,
            None => self.driver.start(&task, &context)?,
        };
        let pointer = self.pointer(turn.session_id());
        self.session.set_agent(id.clone(), pointer.clone());
        self.save()?;
        if previous.is_none() {
            self.dir.append_thread(&ThreadEntry::new(
                ThreadAuthor::Arch,
                ThreadEvent::Subagent {
                    element: id.clone(),
                },
            ))?;
        }

        let mut asked = false;
        let mut names = HashMap::new();
        for event in turn {
            let event = event?;
            if matches!(&event, TurnEvent::ToolCall { name, .. } if name == ASK) {
                asked = true;
            }
            if let Some(mut entry) = thread_entry(id, &event, &mut names) {
                entry.driver = Some(pointer.clone());
                self.dir.append_thread(&entry)?;
            }
        }

        let denied = self
            .dir
            .read_records()?
            .into_iter()
            .skip(records_before)
            .filter_map(|r| match r.kind {
                RecordKind::Denial {
                    element,
                    path,
                    reason,
                    ..
                } if &element == id && cause(&reason) == Cause::Scope => Some(path),
                _ => None,
            })
            .collect();
        Ok(Outcome { asked, denied })
    }

    fn after_turn(&mut self, id: &ElementId, outcome: Outcome) -> Result<(), Error> {
        if outcome.asked {
            self.session.cursor.waiting = Some(Waiting {
                element: id.clone(),
                denied: vec![],
            });
            return self.go(Trigger::Ask);
        }
        if !outcome.denied.is_empty() {
            self.session.cursor.waiting = Some(Waiting {
                element: id.clone(),
                denied: outcome.denied,
            });
            return self.go(Trigger::Denied);
        }
        self.element_mut(id)?.state = ElementState::Done;
        self.dir.write_plan(&self.plan)?;
        self.session.cursor.todo.retain(|e| e != id);
        self.save()
    }

    /// The running group's gate: commands, `arch check`, the judge; then pass, a fix round, or
    /// the four-door question.
    fn gate(&mut self) -> Result<(), Error> {
        let group = self.group()?.clone();
        let runs = gate::run(&group, &self.worktree, &self.dir, self.project)?;
        let mut failing = vec![];
        let mut judged = None;
        if group.gate.judge {
            let pack = self.pack(None, "judge")?;
            let context = Context {
                pack,
                ..Context::default()
            };
            let base = self.session.base.clone().unwrap_or_else(|| "HEAD".into());
            let judging = Judging {
                plan: &self.plan,
                elements: &group.elements,
                runs: &runs,
                worktree: &self.worktree,
                base: &base,
                model: self.options.judge_model.clone(),
            };
            let (verdicts, sid) = judge::run(&mut *self.driver, &judging, &context)?;
            let pointer = self.pointer(&sid);
            for v in &verdicts {
                self.dir.write_record(
                    RecordKind::JudgeVerdict {
                        element: v.element.clone(),
                        verdict: v.verdict,
                        reason: v.reason.clone(),
                    },
                    vec![],
                )?;
                let label = self.element(&v.element)?.label;
                let mut entry = ThreadEntry::new(
                    ThreadAuthor::Judge,
                    ThreadEvent::Text {
                        text: format!("{label} {} · {}", verdict_word(v.verdict), v.reason),
                    },
                );
                entry.driver = Some(pointer.clone());
                self.dir.append_thread(&entry)?;
                if v.verdict == Verdict::Fail {
                    failing.push(v.element.clone());
                }
            }
            let passed = verdicts
                .iter()
                .filter(|v| v.verdict == Verdict::Pass)
                .count();
            judged = Some(format!("judge {passed}/{} pass", verdicts.len()));
        }
        if failing.is_empty() && runs.iter().any(|r| !r.passed()) {
            // A command failed and no verdict says whose fault it is: the whole group goes back.
            failing = group.elements.clone();
        }
        let mut line = format!("gate · {} · {}", group.name, gate::summary(&runs));
        if let Some(j) = judged {
            line.push_str(" · ");
            line.push_str(&j);
        }
        self.arch_line(line)?;

        let last = self.session.cursor.group as usize + 1 >= self.plan.groups.len();
        if failing.is_empty() {
            if last {
                return self.go(Trigger::LastGatePassed);
            }
            self.next_group();
            return self.go(Trigger::GatePassed);
        }
        let rounds = group.gate.fix_rounds + self.session.cursor.extra_rounds;
        self.session.cursor.todo = failing;
        if self.session.cursor.fix_round < rounds {
            self.session.cursor.fix_round += 1;
            return self.go(Trigger::FixRound);
        }
        self.go(Trigger::BudgetSpent)?;
        self.dir.append_thread(&ThreadEntry::new(
            ThreadAuthor::Arch,
            ThreadEvent::Ask {
                questions: vec![Question {
                    text: format!(
                        "The gate of {} still fails after {} fix rounds. What now?",
                        group.name, self.session.cursor.fix_round
                    ),
                    options: FOUR_DOORS.map(String::from).to_vec(),
                }],
            },
        ))?;
        Ok(())
    }

    /// Move the cursor to the next group, with a fresh fix-round budget.
    fn next_group(&mut self) {
        let c = &mut self.session.cursor;
        c.group += 1;
        c.fix_round = 0;
        c.extra_rounds = 0;
        c.todo = self
            .plan
            .groups
            .get(c.group as usize)
            .map(|g| g.elements.clone())
            .unwrap_or_default();
    }

    /// The context pack written for `name` (an element, or `judge`); `None`, with a thread line,
    /// when the facts cannot be had.
    fn pack(&mut self, element: Option<&ElementId>, name: &str) -> Result<Option<PathBuf>, Error> {
        let facts = match self.project.facts(&self.worktree) {
            Ok(f) => f,
            Err(reason) => {
                self.arch_line(format!("no context pack for {name}: {reason}"))?;
                return Ok(None);
            }
        };
        let text = pack::build(
            &facts,
            &ArchDir::of_repo(&self.worktree),
            &self.plan,
            element,
        )?;
        let dir = self.worktree.join(".arch/cache/driver").join(name);
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

    fn element_prompt(&self, e: &Element) -> String {
        ELEMENT_PROMPT
            .replace("{label}", &e.label)
            .replace("{id}", e.id.as_str())
            .replace("{intention}", &e.intention)
            .replace("{site}", &e.site)
            .replace("{files}", &list(&e.files))
            .replace("{plan}", &self.plan.intention)
    }

    /// The fix round's message: the judge's reason for this element, the failing outputs of the
    /// group's last gate run, and the person's hint.
    fn fix_prompt(&self, e: &Element) -> Result<String, Error> {
        let group = self.group()?;
        let records = self.dir.read_records()?;
        let last_run = group.gate.commands.len() + usize::from(group.gate.check);
        let outputs: Vec<String> = records
            .iter()
            .filter_map(|r| match &r.kind {
                RecordKind::GateOutput {
                    group: g,
                    command,
                    exit_code,
                    output,
                } if g == &group.name => Some((command, *exit_code, output)),
                _ => None,
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .take(last_run)
            .rev()
            .filter(|(_, code, _)| *code != 0)
            .map(|(c, code, out)| format!("{c} · exit {code}\n```\n{}\n```", tail(out, 8 * 1024)))
            .collect();
        let verdict = records.iter().rev().find_map(|r| match &r.kind {
            RecordKind::JudgeVerdict {
                element,
                verdict: Verdict::Fail,
                reason,
            } if element == &e.id => Some(reason.clone()),
            _ => None,
        });
        let mut m = format!(
            "The gate of {} failed (fix round {}).",
            group.name, self.session.cursor.fix_round
        );
        if let Some(reason) = verdict {
            m.push_str(&format!("\n\nThe judge on {}: {reason}", e.label));
        }
        if !outputs.is_empty() {
            m.push_str("\n\nFailing gate output:\n\n");
            m.push_str(&outputs.join("\n\n"));
        }
        if let Some(hint) = &self.hint {
            m.push_str(&format!("\n\nThe person's hint: {hint}"));
        }
        m.push_str(
            "\n\nFix the element within its files, then commit again with mcp__arch__commit.",
        );
        Ok(m)
    }

    fn pointer(&self, session_id: &str) -> DriverPointer {
        DriverPointer {
            driver: self.driver.name().to_string(),
            session_id: session_id.to_string(),
            transcript_path: (self.driver.name() == "claude-code")
                .then(|| arch_driver::transcript_path(&self.worktree, session_id))
                .flatten(),
        }
    }

    fn waiting(&self) -> Result<Waiting, Error> {
        self.session
            .cursor
            .waiting
            .clone()
            .ok_or_else(|| Error::NotWaiting(self.session.id.clone()))
    }

    fn group(&self) -> Result<&arch_facts::Group, Error> {
        let i = self.session.cursor.group as usize;
        self.plan.groups.get(i).ok_or(Error::NoGroup(i))
    }

    fn element(&self, id: &ElementId) -> Result<Element, Error> {
        self.plan
            .element(id)
            .cloned()
            .ok_or_else(|| Error::NoElement(id.clone()))
    }

    fn element_mut(&mut self, id: &ElementId) -> Result<&mut Element, Error> {
        self.plan
            .elements
            .iter_mut()
            .find(|e| &e.id == id)
            .ok_or_else(|| Error::NoElement(id.clone()))
    }

    fn go(&mut self, trigger: Trigger) -> Result<(), Error> {
        self.session.state = next(self.session.state, trigger)?;
        self.save()
    }

    fn save(&self) -> Result<(), Error> {
        Ok(self.dir.write_session(&self.session)?)
    }

    fn arch_line(&self, text: String) -> Result<(), Error> {
        Ok(self.dir.append_thread(&ThreadEntry::new(
            ThreadAuthor::Arch,
            ThreadEvent::Text { text },
        ))?)
    }

    fn answer_line(&self, answers: Vec<String>) -> Result<(), Error> {
        Ok(self.dir.append_thread(&ThreadEntry::new(
            ThreadAuthor::You,
            ThreadEvent::Answer { answers },
        ))?)
    }

    /// Commit the session folder when it changed: `chore(arch): <name> · <state>`.
    fn commit_folder(&self) -> Result<(), Error> {
        let folder = format!(".arch/sessions/{}", self.session.id);
        git(&self.worktree, &["add", "--force", "--", &folder])?;
        if git(
            &self.worktree,
            &["diff", "--cached", "--quiet", "--", &folder],
        )
        .is_ok()
        {
            return Ok(());
        }
        let state = serde_json::to_value(self.session.state)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        git(
            &self.worktree,
            &[
                "commit",
                "--quiet",
                "-m",
                &format!("chore(arch): {} · {state}", self.session.name),
                "-m",
                &format!("Arch-Session: {}", self.session.id),
                "--",
                &folder,
            ],
        )?;
        Ok(())
    }
}

fn verdict_word(v: Verdict) -> &'static str {
    match v {
        Verdict::Pass => "pass",
        Verdict::Fail => "fail",
    }
}

fn list(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        return "none".into();
    }
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A driver event as a thread line (ADR 0003: the filtered stream). The start, sub-agents' own
/// starts and hook events are not kept; a tool result names its call's tool.
fn thread_entry(
    element: &ElementId,
    event: &TurnEvent,
    names: &mut HashMap<String, String>,
) -> Option<ThreadEntry> {
    let event = match event {
        TurnEvent::Text { text, .. } => ThreadEvent::Text { text: text.clone() },
        TurnEvent::ToolCall {
            id, name, input, ..
        } => {
            names.insert(id.clone(), name.clone());
            ThreadEvent::ToolCall {
                tool: name.clone(),
                summary: one_line(input),
            }
        }
        TurnEvent::ToolResult {
            id, ok, content, ..
        } => ThreadEvent::ToolResult {
            tool: names.get(id).cloned().unwrap_or_default(),
            ok: *ok,
            summary: short(content),
        },
        TurnEvent::Result(r) => ThreadEvent::Result {
            summary: r.text.clone().unwrap_or_else(|| r.subtype.clone()),
        },
        TurnEvent::Started { .. } | TurnEvent::Subagent { .. } | TurnEvent::Hook { .. } => {
            return None;
        }
    };
    Some(ThreadEntry::new(
        ThreadAuthor::Agent {
            element: Some(element.clone()),
        },
        event,
    ))
}

/// A tool input in one line: its path, command or pattern when it has one.
fn one_line(input: &Value) -> String {
    let named = ["file_path", "path", "command", "pattern"]
        .iter()
        .find_map(|k| input[k].as_str());
    short(&named.map_or_else(|| input.to_string(), String::from))
}

/// The first line, at most 160 characters.
fn short(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    match line.char_indices().nth(160) {
        Some((i, _)) => format!("{}…", &line[..i]),
        None => line.to_string(),
    }
}
