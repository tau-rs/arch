//! The test double: replays recorded stream-json through the real parser (`handoff-arch.md` §4).
//!
//! Each `start` or `resume` plays the next recorded turn and logs the call, so a test of the
//! session engine can assert what it asked for. A recording without a result line ends with
//! [`DriverError::NoResult`], which is how a test simulates an agent that died mid-turn.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::stream::StreamParser;
use crate::{Context, Driver, DriverError, Task, Turn, TurnEvent, TurnHandle};

/// One call the double received.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayCall {
    /// The session resumed, or `None` for a start.
    pub resume: Option<String>,
    /// The task as given.
    pub task: Task,
    /// The context as given.
    pub context: Context,
}

/// A driver that plays recorded turns in order.
#[derive(Debug, Default)]
pub struct ReplayDriver {
    turns: VecDeque<String>,
    calls: Vec<ReplayCall>,
}

impl ReplayDriver {
    /// A double over recorded turns, each the text of one stream-json recording.
    pub fn new(turns: impl IntoIterator<Item = String>) -> Self {
        ReplayDriver {
            turns: turns.into_iter().collect(),
            calls: vec![],
        }
    }

    /// A double over recording files, played in the order given.
    pub fn from_files(
        paths: impl IntoIterator<Item = impl AsRef<Path>>,
    ) -> Result<Self, DriverError> {
        let turns = paths
            .into_iter()
            .map(|p| {
                std::fs::read_to_string(p.as_ref())
                    .map_err(|e| DriverError::Replay(format!("{}: {e}", p.as_ref().display())))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::new(turns))
    }

    /// A double over every `*.jsonl` in a directory, played in file-name order.
    pub fn from_dir(dir: impl AsRef<Path>) -> Result<Self, DriverError> {
        let dir = dir.as_ref();
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| DriverError::Replay(format!("{}: {e}", dir.display())))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        paths.sort();
        Self::from_files(paths)
    }

    /// The calls received so far, in order.
    pub fn calls(&self) -> &[ReplayCall] {
        &self.calls
    }

    /// Recorded turns not yet played.
    pub fn remaining(&self) -> usize {
        self.turns.len()
    }

    fn play(
        &mut self,
        resume: Option<&str>,
        task: &Task,
        context: &Context,
    ) -> Result<Turn, DriverError> {
        self.calls.push(ReplayCall {
            resume: resume.map(String::from),
            task: task.clone(),
            context: context.clone(),
        });
        let text = self
            .turns
            .pop_front()
            .ok_or_else(|| DriverError::Replay("no recorded turn left".into()))?;
        let mut parser = StreamParser::default();
        let events: Vec<TurnEvent> = text.lines().flat_map(|l| parser.line(l)).collect();
        let recorded_id = events.iter().find_map(|e| match e {
            TurnEvent::Started { session_id, .. } => Some(session_id.clone()),
            TurnEvent::Result(r) => Some(r.session_id.clone()),
            _ => None,
        });
        let session_id = match resume {
            Some(id) => id.to_string(),
            None => recorded_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        };
        let mut items: Vec<Result<TurnEvent, DriverError>> = events.into_iter().map(Ok).collect();
        if parser.stats().results == 0 {
            items.push(Err(DriverError::NoResult));
        }
        Ok(Turn::new(
            session_id,
            Box::new(items.into_iter()),
            TurnHandle::default(),
        ))
    }
}

impl Driver for ReplayDriver {
    fn name(&self) -> &'static str {
        "replay"
    }

    fn start(&mut self, task: &Task, context: &Context) -> Result<Turn, DriverError> {
        self.play(None, task, context)
    }

    fn resume(
        &mut self,
        session_id: &str,
        task: &Task,
        context: &Context,
    ) -> Result<Turn, DriverError> {
        self.play(Some(session_id), task, context)
    }
}
