//! The sqlite store under `.arch/cache/` (ADR 0001): facts by commit hash as per-file deltas
//! with content hashes, branch → head pointers, worktree-state hashes (ADR 0002), view cache
//! tables, plan drafts (ADR 0020).
//!
//! No other crate opens the database; this module is the only place SQL lives.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Error, Result};
use crate::hash::{ContentHash, TreeKey};
use serde::{Deserialize, Serialize};

use crate::model::{
    Analyzer, Commit, Crate, Entry, External, Facts, Item, Link, Port, Repo, Table,
};
use crate::session::{Plan, now};

const SCHEMA: &str = include_str!("schema.sql");
/// Bumped when `schema.sql` changes incompatibly; the cache is then rebuilt (ADR 0001: a clone
/// without the cache is complete).
pub const STORE_SCHEMA_VERSION: i64 = 1;

/// The file name of the store inside `.arch/cache/`.
pub const STORE_FILE: &str = "facts.sqlite";

/// A tree's file list: path → content hash.
pub type TreeFiles = Vec<(PathBuf, ContentHash)>;

/// The facts of one file at one content hash: the unit of storage and recompute (ADR 0002).
/// A tree's [`Facts`] are the union of its files' deltas under its [`TreeHead`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileFacts {
    /// Path, relative to the repo root.
    pub path: PathBuf,
    /// Content hash of the file these facts were computed from.
    pub file_hash: ContentHash,
    /// Items declared in the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Item>,
    /// Links whose `from` is in the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    /// Ports declared in the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<Port>,
    /// Externals first touched in the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub externals: Vec<External>,
    /// Entries in the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<Entry>,
    /// Tables, when the file is a migration (ADR 0008).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<Table>,
    /// Set when these facts are syntax-level only (ADR 0010), with the reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<String>,
}

impl FileFacts {
    /// Empty facts for a file at a hash.
    pub fn empty(path: impl Into<PathBuf>, file_hash: ContentHash) -> Self {
        FileFacts {
            path: path.into(),
            file_hash,
            items: vec![],
            links: vec![],
            ports: vec![],
            externals: vec![],
            entries: vec![],
            tables: vec![],
            degraded: None,
        }
    }
}

/// What a tree's facts carry besides its per-file deltas: the repo and unit, the analyzer
/// and its degraded crates, the crates seen (ADR 0007, 0010).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreeHead {
    /// The repository, commit (or `wt:` state hash) and unit.
    pub repo: Repo,
    /// The analyzer that produced the facts.
    pub analyzer: Analyzer,
    /// Crates seen, analyzed or not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crates: Vec<Crate>,
}

/// The store.
pub struct Store {
    conn: Connection,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.conn.path())
            .finish()
    }
}

impl Store {
    /// Open (creating) the store of an `.arch/` directory: `<arch_dir>/cache/facts.sqlite`.
    pub fn open(arch_dir: &Path) -> Result<Self> {
        let cache = arch_dir.join("cache");
        std::fs::create_dir_all(&cache).map_err(|e| Error::io(&cache, e))?;
        Self::open_at(&cache.join(STORE_FILE))
    }

    /// Open (creating) a store at an explicit path.
    pub fn open_at(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// An in-memory store, for tests and drafts that must leave no trace.
    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        let version: Option<i64> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get::<_, String>(0).map(|s| s.parse().unwrap_or(0)),
            )
            .optional()
            .unwrap_or(None);
        match version {
            Some(v) if v == STORE_SCHEMA_VERSION => {}
            Some(_) => {
                // Older or newer schema: the cache is disposable (ADR 0001); rebuild.
                drop_everything(&conn)?;
                conn.execute_batch(SCHEMA)?;
            }
            None => conn.execute_batch(SCHEMA)?,
        }
        conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![STORE_SCHEMA_VERSION.to_string()],
        )?;
        Ok(Store { conn })
    }

    // ----- per-file deltas (ADR 0002) -----

    /// Store one file's facts, keyed by its content hash. Idempotent.
    pub fn put_file_facts(&self, facts: &FileFacts) -> Result<()> {
        let json = serde_json::to_string(facts)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO file_facts(file_hash, path, facts, degraded) VALUES (?1, ?2, ?3, ?4)",
            params![facts.file_hash.as_str(), path_str(&facts.path), json, facts.degraded],
        )?;
        Ok(())
    }

    /// The facts of a file at a content hash, if computed.
    pub fn file_facts(&self, file_hash: &ContentHash) -> Result<Option<FileFacts>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT facts FROM file_facts WHERE file_hash = ?1",
                params![file_hash.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match json {
            Some(j) => Some(serde_json::from_str(&j)?),
            None => None,
        })
    }

    /// Whether a file's facts at this hash are already known (a rebase reuses them).
    pub fn has_file_facts(&self, file_hash: &ContentHash) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM file_facts WHERE file_hash = ?1",
            params![file_hash.as_str()],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    // ----- trees: a commit or a worktree state -----

    /// Record a tree: which files at which hashes, its head, and the commits on its branch.
    /// Replaces any earlier record of the same key.
    pub fn put_tree(
        &mut self,
        key: &TreeKey,
        base_commit: Option<&str>,
        files: &[(PathBuf, ContentHash)],
        head: &TreeHead,
        commits: &[String],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let k = key.db_key();
        tx.execute("DELETE FROM trees WHERE key = ?1", params![k])?;
        tx.execute(
            "INSERT INTO trees(key, kind, base_commit, head, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![k, key.kind(), base_commit, serde_json::to_string(head)?, now().to_string()],
        )?;
        {
            let mut ins = tx
                .prepare("INSERT INTO tree_files(tree_key, path, file_hash) VALUES (?1, ?2, ?3)")?;
            for (p, h) in files {
                ins.execute(params![k, path_str(p), h.as_str()])?;
            }
            let mut insc =
                tx.prepare("INSERT INTO tree_commits(tree_key, ord, hash) VALUES (?1, ?2, ?3)")?;
            for (i, h) in commits.iter().enumerate() {
                insc.execute(params![k, i as i64, h])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Whether a tree is recorded.
    pub fn has_tree(&self, key: &TreeKey) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM trees WHERE key = ?1",
            params![key.db_key()],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// The files of a tree.
    pub fn tree_files(&self, key: &TreeKey) -> Result<TreeFiles> {
        let mut st = self
            .conn
            .prepare("SELECT path, file_hash FROM tree_files WHERE tree_key = ?1 ORDER BY path")?;
        let rows = st.query_map(params![key.db_key()], |r| {
            Ok((
                PathBuf::from(r.get::<_, String>(0)?),
                ContentHash(r.get(1)?),
            ))
        })?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(Into::into)
    }

    /// The files of a tree whose facts are not yet computed: what the analyzer must do.
    pub fn missing_file_facts(&self, key: &TreeKey) -> Result<TreeFiles> {
        let mut st = self.conn.prepare(
            "SELECT t.path, t.file_hash FROM tree_files t LEFT JOIN file_facts f ON f.file_hash = t.file_hash
             WHERE t.tree_key = ?1 AND f.file_hash IS NULL ORDER BY t.path",
        )?;
        let rows = st.query_map(params![key.db_key()], |r| {
            Ok((
                PathBuf::from(r.get::<_, String>(0)?),
                ContentHash(r.get(1)?),
            ))
        })?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(Into::into)
    }

    /// The files that differ between two trees (added, removed or changed in `b`): a
    /// worktree's facts are its base's plus these deltas (ADR 0002).
    pub fn changed_files(&self, a: &TreeKey, b: &TreeKey) -> Result<Vec<PathBuf>> {
        let mut st = self.conn.prepare(
            "SELECT path FROM (
               SELECT path, file_hash FROM tree_files WHERE tree_key = ?1
               UNION ALL
               SELECT path, file_hash FROM tree_files WHERE tree_key = ?2)
             GROUP BY path HAVING count(DISTINCT file_hash) <> 1 OR count(*) <> 2
             ORDER BY path",
        )?;
        let rows = st.query_map(params![a.db_key(), b.db_key()], |r| {
            Ok(PathBuf::from(r.get::<_, String>(0)?))
        })?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(Into::into)
    }

    /// A tree's head.
    pub fn tree_head(&self, key: &TreeKey) -> Result<Option<TreeHead>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT head FROM trees WHERE key = ?1",
                params![key.db_key()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match json {
            Some(j) => Some(serde_json::from_str(&j)?),
            None => None,
        })
    }

    /// Assemble a tree's facts from its head, its per-file deltas and its commits, sorted per
    /// the golden stability rules (`docs/arch-facts.md`). Files whose facts are missing are
    /// skipped: call [`Store::missing_file_facts`] first to know. `None` for an unknown tree.
    pub fn facts(&self, key: &TreeKey) -> Result<Option<Facts>> {
        let Some(head) = self.tree_head(key)? else {
            return Ok(None);
        };
        let k = key.db_key();
        let mut facts = Facts::empty(head.repo, head.analyzer);
        facts.crates = head.crates;
        let mut st = self.conn.prepare(
            "SELECT f.facts FROM tree_files t JOIN file_facts f ON f.file_hash = t.file_hash WHERE t.tree_key = ?1",
        )?;
        for json in st.query_map(params![k], |r| r.get::<_, String>(0))? {
            let file: FileFacts = serde_json::from_str(&json?)?;
            facts.items.extend(file.items);
            facts.links.extend(file.links);
            facts.ports.extend(file.ports);
            facts.externals.extend(file.externals);
            facts.entries.extend(file.entries);
            facts.tables.extend(file.tables);
        }
        let mut stc = self.conn.prepare(
            "SELECT c.fact FROM tree_commits tc JOIN commits c ON c.hash = tc.hash WHERE tc.tree_key = ?1 ORDER BY tc.ord",
        )?;
        for json in stc.query_map(params![k], |r| r.get::<_, String>(0))? {
            facts.commits.push(serde_json::from_str(&json?)?);
        }
        normalize(&mut facts);
        Ok(Some(facts))
    }

    // ----- commits as facts (spec §7) -----

    /// Store commits, keyed by hash. Idempotent.
    pub fn put_commits(&mut self, commits: &[Commit]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut ins =
                tx.prepare("INSERT OR REPLACE INTO commits(hash, fact) VALUES (?1, ?2)")?;
            for c in commits {
                ins.execute(params![c.hash, serde_json::to_string(c)?])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// One commit.
    pub fn commit(&self, hash: &str) -> Result<Option<Commit>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT fact FROM commits WHERE hash = ?1",
                params![hash],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match json {
            Some(j) => Some(serde_json::from_str(&j)?),
            None => None,
        })
    }

    // ----- pointers (ADR 0002): branches and worktrees -----

    /// Point a branch at a commit.
    pub fn set_branch_head(&self, branch: &str, head: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO branches(name, head) VALUES (?1, ?2)",
            params![branch, head],
        )?;
        Ok(())
    }

    /// A branch's head commit.
    pub fn branch_head(&self, branch: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT head FROM branches WHERE name = ?1",
                params![branch],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Record a worktree: its current state hash, base commit and branch.
    pub fn set_worktree(
        &self,
        path: &Path,
        state: &ContentHash,
        base_commit: &str,
        branch: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO worktrees(path, state_hash, base_commit, branch) VALUES (?1, ?2, ?3, ?4)",
            params![path_str(path), state.as_str(), base_commit, branch],
        )?;
        Ok(())
    }

    /// A worktree's record: (state hash, base commit, branch).
    pub fn worktree(&self, path: &Path) -> Result<Option<(ContentHash, String, Option<String>)>> {
        self.conn
            .query_row(
                "SELECT state_hash, base_commit, branch FROM worktrees WHERE path = ?1",
                params![path_str(path)],
                |r| Ok((ContentHash(r.get(0)?), r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Forget a worktree (removed at archive, ADR 0003).
    pub fn remove_worktree(&self, path: &Path) -> Result<()> {
        self.conn.execute(
            "DELETE FROM worktrees WHERE path = ?1",
            params![path_str(path)],
        )?;
        Ok(())
    }

    // ----- view cache, per branch (handoff-arch.md §2 arch-views) -----

    /// Cache a view's payload for a branch at a tree.
    pub fn view_cache_put(
        &self,
        branch: &str,
        view: &str,
        key: &str,
        tree: &TreeKey,
        payload: &[u8],
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO view_cache(branch, view, key, tree_key, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![branch, view, key, tree.db_key(), payload],
        )?;
        Ok(())
    }

    /// A cached view payload, with the tree it was computed at; the caller decides whether
    /// that tree is still current.
    pub fn view_cache_get(
        &self,
        branch: &str,
        view: &str,
        key: &str,
    ) -> Result<Option<(TreeKey, Vec<u8>)>> {
        let row: Option<(String, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT tree_key, payload FROM view_cache WHERE branch = ?1 AND view = ?2 AND key = ?3",
                params![branch, view, key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.and_then(|(t, p)| TreeKey::from_db_key(&t).map(|t| (t, p))))
    }

    /// Drop a branch's cached views.
    pub fn view_cache_invalidate(&self, branch: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM view_cache WHERE branch = ?1", params![branch])?;
        Ok(())
    }

    // ----- plan drafts (ADR 0020) -----

    /// Save a draft; it lives here until Accept.
    pub fn plan_draft_put(&self, plan: &Plan) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO plan_drafts(id, plan, updated_at) VALUES (?1, ?2, ?3)",
            params![
                plan.session.as_str(),
                serde_json::to_string(plan)?,
                now().to_string()
            ],
        )?;
        Ok(())
    }

    /// A draft by session id.
    pub fn plan_draft(&self, session: &str) -> Result<Option<Plan>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT plan FROM plan_drafts WHERE id = ?1",
                params![session],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match json {
            Some(j) => Some(serde_json::from_str(&j)?),
            None => None,
        })
    }

    /// Discard deletes (ADR 0020); Accept moves the draft out of the cache too.
    pub fn plan_draft_delete(&self, session: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM plan_drafts WHERE id = ?1", params![session])?;
        Ok(())
    }

    /// The ids of all drafts.
    pub fn plan_drafts(&self) -> Result<Vec<String>> {
        let mut st = self
            .conn
            .prepare("SELECT id FROM plan_drafts ORDER BY updated_at")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(Into::into)
    }
}

/// Sort every array by its stable key (`docs/arch-facts.md`): items, ports, externals by id;
/// tables, crates by name; links by from, to, kind, member; entries by item; commits keep history order.
fn normalize(facts: &mut Facts) {
    facts.crates.sort_by(|a, b| a.name.cmp(&b.name));
    facts.items.sort_by(|a, b| a.id.cmp(&b.id));
    facts.links.sort_by_key(|l| {
        (
            l.from.clone(),
            format!("{:?}", l.to),
            format!("{:?}", l.kind),
            l.member.clone(),
        )
    });
    facts.ports.sort_by(|a, b| a.id.cmp(&b.id));
    facts.externals.sort_by(|a, b| a.id.cmp(&b.id));
    facts.entries.sort_by(|a, b| a.item.cmp(&b.item));
    facts.tables.sort_by(|a, b| a.name.cmp(&b.name));
}

fn drop_everything(conn: &Connection) -> Result<()> {
    let names: Vec<String> = {
        let mut st = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        )?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    for n in names {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{n}\""))?;
    }
    Ok(())
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}
