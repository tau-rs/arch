//! `cache/tool-layer/<element>.json` (ADR 0012, tau-rs/arch-design#34): the tool layer's state
//! for one element of one session. Gitignored, never committed.
//!
//! `arch hook pre|post` and the MCP `read` tool write it (arch-driver); the watcher reads it to
//! tell the session's writes from yours (arch-analyze). The shape:
//!
//! ```json
//! { "session": "s-…", "element": "e-…",
//!   "expected": [ { "path": "src/pay.rs", "sha256": "…" } ],
//!   "read": { "src/pay.rs": "…" },
//!   "writes": [ { "path": "src/pay.rs", "sha256": "…", "at": "2026-10-03T…Z" } ] }
//! ```
//!
//! - `expected`: writes the pre hook let through, by content; the post hook confirms each with
//!   the hash on disk, or drops it when the tool failed. The watcher reads only this list.
//! - `read`: each file's hash when the element's agent last saw it (a read, or its own write):
//!   the stale-write guard compares the file on disk against it.
//! - `writes`: the attribution log, one entry per confirmed write.
//!
//! Paths are relative to the worktree. Every update holds an exclusive lock on a sidecar
//! `<element>.lock` and replaces the file atomically, because hooks and the MCP server are
//! separate processes that may run at once.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::hash::ContentHash;
use crate::session::{ElementId, SessionId, Timestamp, now};

/// A write the tool layer let through, by content (ADR 0012).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedWrite {
    /// Worktree-relative path.
    pub path: PathBuf,
    /// The content's sha256 once written.
    pub sha256: ContentHash,
}

/// One confirmed write, attributed to the element (spec §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributedWrite {
    /// Worktree-relative path.
    pub path: PathBuf,
    /// The content's sha256 after the write.
    pub sha256: ContentHash,
    /// When the post hook confirmed it.
    pub at: Timestamp,
}

/// The tool layer's state for one element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolLayerState {
    /// The session.
    pub session: SessionId,
    /// The element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<ElementId>,
    /// Writes let through, waiting for (or confirmed by) the post hook; what the watcher reads.
    #[serde(default)]
    pub expected: Vec<ExpectedWrite>,
    /// Each file's hash when the agent last saw it.
    #[serde(default)]
    pub read: BTreeMap<PathBuf, ContentHash>,
    /// Confirmed writes, in order.
    #[serde(default)]
    pub writes: Vec<AttributedWrite>,
}

impl ToolLayerState {
    /// An empty state for an element of a session.
    pub fn new(session: SessionId, element: ElementId) -> Self {
        ToolLayerState {
            session,
            element: Some(element),
            expected: vec![],
            read: BTreeMap::new(),
            writes: vec![],
        }
    }

    /// The hash of `path` when the agent last saw it.
    pub fn last_read(&self, path: &Path) -> Option<&ContentHash> {
        self.read.get(path)
    }

    /// The agent saw `path` with content `hash`.
    pub fn record_read(&mut self, path: &Path, hash: ContentHash) {
        self.read.insert(path.to_path_buf(), hash);
    }

    /// The pre hook let a write through: `path` will hold content `hash`.
    pub fn expect(&mut self, path: &Path, hash: ContentHash) {
        if !self.matches(path, &hash) {
            self.expected.push(ExpectedWrite {
                path: path.to_path_buf(),
                sha256: hash,
            });
        }
    }

    /// The write to `path` landed with content `hash` on disk: it is the only expected content of
    /// that path now, the agent has seen it, and the write is attributed.
    pub fn confirm(&mut self, path: &Path, hash: ContentHash) {
        self.expected.retain(|e| e.path != path || e.sha256 == hash);
        self.expect(path, hash.clone());
        self.record_read(path, hash.clone());
        self.writes.push(AttributedWrite {
            path: path.to_path_buf(),
            sha256: hash,
            at: now(),
        });
    }

    /// The write to `path` failed: keep only an expectation the file on disk (`on_disk`, `None`
    /// when absent) still matches.
    pub fn drop_failed(&mut self, path: &Path, on_disk: Option<&ContentHash>) {
        self.expected
            .retain(|e| e.path != path || Some(&e.sha256) == on_disk);
    }

    /// Whether a write of `hash` to `path` is expected (the watcher's question).
    pub fn matches(&self, path: &Path, hash: &ContentHash) -> bool {
        self.expected
            .iter()
            .any(|e| e.path == path && &e.sha256 == hash)
    }
}

/// The directory holding every element's state: `<worktree>/.arch/cache/tool-layer/`.
pub fn tool_layer_dir(worktree: &Path) -> PathBuf {
    worktree.join(".arch/cache/tool-layer")
}

/// Every readable state under a worktree, by file name. A file that does not parse claims
/// nothing and is skipped.
pub fn tool_layer_states(worktree: &Path) -> Vec<ToolLayerState> {
    let Ok(entries) = std::fs::read_dir(tool_layer_dir(worktree)) else {
        return vec![];
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .filter_map(|text| serde_json::from_str(&text).ok())
        .collect()
}

/// One element's state file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLayerFile {
    path: PathBuf,
    session: SessionId,
    element: ElementId,
}

impl ToolLayerFile {
    /// The state file of `element` (of `session`) in `worktree`.
    pub fn new(worktree: &Path, session: SessionId, element: ElementId) -> Self {
        ToolLayerFile {
            path: tool_layer_dir(worktree).join(format!("{}.json", element.as_str())),
            session,
            element,
        }
    }

    /// The file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The current state; empty when the file does not exist yet.
    pub fn load(&self) -> Result<ToolLayerState> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                serde_json::from_str(&text).map_err(|e| Error::format(&self.path, e.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ToolLayerState::new(
                self.session.clone(),
                self.element.clone(),
            )),
            Err(e) => Err(Error::io(&self.path, e)),
        }
    }

    /// Read, change and write back the state under the element's lock; returns what `change`
    /// returned.
    pub fn update<T>(&self, change: impl FnOnce(&mut ToolLayerState) -> T) -> Result<T> {
        let dir = self.path.parent().expect("under the tool-layer dir");
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        let lock_path = self.path.with_extension("lock");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|e| Error::io(&lock_path, e))?;
        lock.lock().map_err(|e| Error::io(&lock_path, e))?;
        let mut state = self.load()?;
        let out = change(&mut state);
        let tmp = self.path.with_extension("json.tmp");
        let mut text = serde_json::to_string_pretty(&state)?;
        text.push('\n');
        std::fs::File::create(&tmp)
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .map_err(|e| Error::io(&tmp, e))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| Error::io(&self.path, e))?;
        // Dropping `lock` releases it.
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> ToolLayerState {
        ToolLayerState::new(SessionId::new("s-1"), ElementId::from_str_unchecked("e-1"))
    }

    #[test]
    fn confirm_keeps_only_the_content_on_disk_and_attributes_it() {
        let p = Path::new("src/pay.rs");
        let (a, b) = (ContentHash::of_str("a"), ContentHash::of_str("b"));
        let mut s = state();
        s.expect(p, a.clone());
        s.expect(p, a.clone());
        s.expect(p, b.clone());
        assert_eq!(s.expected.len(), 2);
        s.confirm(p, b.clone());
        assert!(s.matches(p, &b) && !s.matches(p, &a));
        assert_eq!(s.last_read(p), Some(&b));
        assert_eq!(s.writes.len(), 1);
    }

    #[test]
    fn a_failed_write_drops_what_the_disk_does_not_hold() {
        let p = Path::new("src/pay.rs");
        let (a, b) = (ContentHash::of_str("a"), ContentHash::of_str("b"));
        let mut s = state();
        s.confirm(p, a.clone());
        s.expect(p, b.clone());
        s.drop_failed(p, Some(&a));
        assert!(s.matches(p, &a) && !s.matches(p, &b));
        s.drop_failed(p, None);
        assert!(s.expected.is_empty());
        assert!(s.writes.len() == 1, "a failure attributes nothing");
    }

    #[test]
    fn the_file_round_trips_under_its_lock_and_reads_back_for_the_watcher() {
        let tmp = tempfile::tempdir().unwrap();
        let file = ToolLayerFile::new(
            tmp.path(),
            SessionId::new("s-1"),
            ElementId::from_str_unchecked("e-1"),
        );
        assert_eq!(file.load().unwrap(), state());
        let h = ContentHash::of_str("x");
        file.update(|s| s.expect(Path::new("a.rs"), h.clone()))
            .unwrap();
        let states = tool_layer_states(tmp.path());
        assert_eq!(states.len(), 1);
        assert!(states[0].matches(Path::new("a.rs"), &h));
        assert!(!file.path().with_extension("json.tmp").exists());
    }

    #[test]
    fn the_watchers_provisional_shape_still_reads() {
        let s: ToolLayerState = serde_json::from_str(
            r#"{"session":"s-1","element":"e-1","expected":[{"path":"src/pay.rs","sha256":"ab"}]}"#,
        )
        .unwrap();
        assert!(s.matches(Path::new("src/pay.rs"), &ContentHash("ab".into())));
    }
}
