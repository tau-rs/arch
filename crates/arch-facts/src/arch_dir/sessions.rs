//! `sessions/<id>/` (ADR 0003): `session.toml`, `plan.toml`, `thread.jsonl`, `records/`.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::model::Witness;
use crate::session::{Plan, Record, RecordKind, Session, SessionId, ThreadEntry, now};

/// A session's folder on its branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDir {
    root: PathBuf,
    id: SessionId,
}

impl SessionDir {
    pub(crate) fn new(root: PathBuf, id: SessionId) -> Self {
        SessionDir { root, id }
    }

    /// The session id.
    pub fn id(&self) -> &SessionId {
        &self.id
    }
    /// The folder.
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// `session.toml`: the session record (ADR 0015).
    pub fn session_path(&self) -> PathBuf {
        self.root.join("session.toml")
    }
    /// `plan.toml`.
    pub fn plan_path(&self) -> PathBuf {
        self.root.join("plan.toml")
    }
    /// `thread.jsonl`.
    pub fn thread_path(&self) -> PathBuf {
        self.root.join("thread.jsonl")
    }
    /// `records/`.
    pub fn records_dir(&self) -> PathBuf {
        self.root.join("records")
    }

    /// Whether the folder exists on this branch.
    pub fn exists(&self) -> bool {
        self.root.is_dir()
    }

    fn ensure(&self) -> Result<()> {
        std::fs::create_dir_all(self.records_dir()).map_err(|e| Error::io(self.records_dir(), e))
    }

    /// Write `plan.toml` (Accept, and every reconcile; ADR 0021: the plan owns the elements).
    pub fn write_plan(&self, plan: &Plan) -> Result<()> {
        self.ensure()?;
        let text = toml::to_string_pretty(plan)
            .map_err(|e| Error::Other(anyhow::anyhow!("plan.toml: {e}")))?;
        std::fs::write(self.plan_path(), text).map_err(|e| Error::io(self.plan_path(), e))
    }

    /// Read `plan.toml`; `None` when the branch has no plan (ADR 0022: `plan · none`).
    pub fn read_plan(&self) -> Result<Option<Plan>> {
        let p = self.plan_path();
        match std::fs::read_to_string(&p) {
            Ok(s) => toml::from_str(&s)
                .map(Some)
                .map_err(|e| Error::format(&p, e.message())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(p, e)),
        }
    }

    /// Write `session.toml`: Accept, then after every scheduler step (ADR 0015).
    pub fn write_session(&self, session: &Session) -> Result<()> {
        self.ensure()?;
        let text = toml::to_string_pretty(session)
            .map_err(|e| Error::Other(anyhow::anyhow!("session.toml: {e}")))?;
        let p = self.session_path();
        let tmp = self.root.join(".session.toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| Error::io(&tmp, e))?;
        std::fs::rename(&tmp, &p).map_err(|e| Error::io(p, e))
    }

    /// Read `session.toml`; `None` before Accept.
    pub fn read_session(&self) -> Result<Option<Session>> {
        let p = self.session_path();
        match std::fs::read_to_string(&p) {
            Ok(s) => toml::from_str(&s)
                .map(Some)
                .map_err(|e| Error::format(&p, e.message())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(p, e)),
        }
    }

    /// Append one line to `thread.jsonl`.
    pub fn append_thread(&self, entry: &ThreadEntry) -> Result<()> {
        self.ensure()?;
        let p = self.thread_path();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&p)
            .map_err(|e| Error::io(&p, e))?;
        let mut line = serde_json::to_string(entry)?;
        line.push('\n');
        f.write_all(line.as_bytes()).map_err(|e| Error::io(&p, e))
    }

    /// Read the whole thread.
    pub fn read_thread(&self) -> Result<Vec<ThreadEntry>> {
        let p = self.thread_path();
        let text = match std::fs::read_to_string(&p) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(Error::io(p, e)),
        };
        text.lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty())
            .map(|(i, l)| {
                serde_json::from_str(l)
                    .map_err(|e| Error::format(&p, format!("line {}: {e}", i + 1)))
            })
            .collect()
    }

    /// Write a record as the next file under `records/`: `NNNN-<kind>.toml`. Returns it.
    pub fn write_record(&self, kind: RecordKind, witnesses: Vec<Witness>) -> Result<Record> {
        self.ensure()?;
        let seq = self.read_records()?.last().map(|r| r.seq + 1).unwrap_or(1);
        let record = Record {
            seq,
            at: now(),
            kind,
            witnesses,
        };
        let p = self
            .records_dir()
            .join(format!("{seq:04}-{}.toml", record.kind.name()));
        let text = toml::to_string_pretty(&record)
            .map_err(|e| Error::Other(anyhow::anyhow!("record: {e}")))?;
        std::fs::write(&p, text).map_err(|e| Error::io(&p, e))?;
        Ok(record)
    }

    /// Read every record, in sequence order.
    pub fn read_records(&self) -> Result<Vec<Record>> {
        let dir = self.records_dir();
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(Error::io(&dir, e)),
        };
        let mut records = vec![];
        for entry in rd {
            let p = entry.map_err(|e| Error::io(&dir, e))?.path();
            if p.extension().is_some_and(|x| x == "toml") {
                let s = std::fs::read_to_string(&p).map_err(|e| Error::io(&p, e))?;
                let r: Record = toml::from_str(&s).map_err(|e| Error::format(&p, e.message()))?;
                records.push(r);
            }
        }
        records.sort_by_key(|r| r.seq);
        Ok(records)
    }

    /// The files of the folder as (relative path, content), for the archive (ADR 0003).
    pub fn files(&self) -> Result<Vec<(PathBuf, String)>> {
        let mut out = vec![];
        if !self.exists() {
            return Ok(out);
        }
        walk(&self.root, &self.root, &mut out)?;
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    /// Remove the folder (archive moves it to notes, ADR 0003).
    pub fn remove(&self) -> Result<()> {
        if self.exists() {
            std::fs::remove_dir_all(&self.root).map_err(|e| Error::io(&self.root, e))?;
        }
        Ok(())
    }
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<(PathBuf, String)>) -> Result<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))? {
        let p = entry.map_err(|e| Error::io(dir, e))?.path();
        if p.is_dir() {
            walk(base, &p, out)?;
        } else {
            let rel = p.strip_prefix(base).expect("under base").to_path_buf();
            let s = std::fs::read_to_string(&p).map_err(|e| Error::io(&p, e))?;
            out.push((rel, s));
        }
    }
    Ok(())
}
