-- arch-facts store (ADR 0001, 0002, 0020). Owned by arch-facts; no other crate opens it.

CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- One file's facts: the per-file delta, named by path · content hash · package id (ADR 0002).
CREATE TABLE IF NOT EXISTS file_facts (
  facts_key TEXT PRIMARY KEY,
  path      TEXT NOT NULL,
  file_hash TEXT NOT NULL,
  facts     TEXT NOT NULL,          -- FileFacts as JSON
  degraded  TEXT                    -- reason when syntax-level only (ADR 0010)
);

-- A tree: a commit, or an uncommitted worktree state (ADR 0002).
CREATE TABLE IF NOT EXISTS trees (
  key         TEXT PRIMARY KEY,     -- "commit:<hash>" | "worktree:<state-hash>"
  kind        TEXT NOT NULL CHECK (kind IN ('commit', 'worktree')),
  base_commit TEXT,                 -- for a worktree: the commit it diverges from
  head        TEXT NOT NULL,        -- TreeHead as JSON: repo, analyzer, crates (ADR 0007, 0010)
  created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tree_files (
  tree_key  TEXT NOT NULL REFERENCES trees(key) ON DELETE CASCADE,
  path      TEXT NOT NULL,
  file_hash TEXT NOT NULL,          -- what changed_files compares
  facts_key TEXT NOT NULL,          -- which delta holds the file's facts
  PRIMARY KEY (tree_key, path)
);
CREATE INDEX IF NOT EXISTS tree_files_by_facts ON tree_files(facts_key);

-- Commits as facts (spec §7), and which tree's branch they are on.
CREATE TABLE IF NOT EXISTS commits (
  hash TEXT PRIMARY KEY,
  fact TEXT NOT NULL                -- Commit as JSON
);

CREATE TABLE IF NOT EXISTS tree_commits (
  tree_key TEXT NOT NULL REFERENCES trees(key) ON DELETE CASCADE,
  ord      INTEGER NOT NULL,
  hash     TEXT NOT NULL,
  PRIMARY KEY (tree_key, ord)
);

-- Pointers (ADR 0002).
CREATE TABLE IF NOT EXISTS branches (
  name TEXT PRIMARY KEY,
  head TEXT NOT NULL
);

-- A worktree's current tree: while one points at a worktree state, that state is kept.
CREATE TABLE IF NOT EXISTS worktrees (
  path        TEXT PRIMARY KEY,
  tree_key    TEXT NOT NULL,        -- "commit:<hash>" | "worktree:<state-hash>"
  base_commit TEXT,
  branch      TEXT
);

-- View cache, per branch (arch-views).
CREATE TABLE IF NOT EXISTS view_cache (
  branch   TEXT NOT NULL,
  view     TEXT NOT NULL,
  key      TEXT NOT NULL,
  tree_key TEXT NOT NULL,
  payload  BLOB NOT NULL,
  PRIMARY KEY (branch, view, key)
);

-- Plan drafts, until Accept (ADR 0020).
CREATE TABLE IF NOT EXISTS plan_drafts (
  id         TEXT PRIMARY KEY,
  plan       TEXT NOT NULL,         -- Plan as JSON
  updated_at TEXT NOT NULL
);
