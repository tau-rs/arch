//! Content hashes and tree keys (ADR 0002).
//!
//! Facts are keyed by commit hash as per-file deltas; uncommitted work is keyed by a
//! worktree-state hash; a delta is named by its file's path, content hash and package id
//! ([`FactsKey`]), and a rebase reuses the deltas whose key is unchanged.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A sha256 hex digest of some bytes: a file's content, or a worktree state.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash(pub String);

impl ContentHash {
    /// Hash raw bytes.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        ContentHash(hex::encode(Sha256::digest(bytes)))
    }

    /// Hash a string's UTF-8 bytes.
    pub fn of_str(s: &str) -> Self {
        Self::of_bytes(s.as_bytes())
    }

    /// The worktree-state hash of a set of files (ADR 0002): sha256 over the sorted
    /// `path NUL file-hash NL` lines, so the same tree always keys the same facts.
    pub fn of_worktree_state<'a, I>(files: I) -> Self
    where
        I: IntoIterator<Item = (&'a Path, &'a ContentHash)>,
    {
        let mut lines: Vec<String> = files
            .into_iter()
            .map(|(p, h)| format!("{}\0{}\n", p.to_string_lossy(), h.0))
            .collect();
        lines.sort();
        let mut hasher = Sha256::new();
        for l in &lines {
            hasher.update(l.as_bytes());
        }
        ContentHash(hex::encode(hasher.finalize()))
    }

    /// The hex digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What names one file's facts (ADR 0002): a hash of its path, its content hash and its package
/// id. The package id names everything else the facts depend on (the package's git tree id, the
/// packages it depends on, `Cargo.lock`, the analyzer), so two trees share a file's delta only
/// when it is the same delta.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FactsKey(pub String);

impl FactsKey {
    /// The key of the facts of the file at `path`, with content `file_hash`, in a package whose
    /// id is `package`.
    pub fn of(path: &Path, file_hash: &ContentHash, package: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(path.to_string_lossy().replace('\\', "/").as_bytes());
        hasher.update([0]);
        hasher.update(file_hash.0.as_bytes());
        hasher.update([0]);
        hasher.update(package.as_bytes());
        FactsKey(hex::encode(hasher.finalize()))
    }

    /// The hex digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FactsKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a set of facts is keyed by (ADR 0002): a commit, or an uncommitted worktree state.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "hash", rename_all = "kebab-case")]
pub enum TreeKey {
    /// A commit hash.
    Commit(String),
    /// A worktree-state hash of uncommitted work.
    Worktree(ContentHash),
}

impl TreeKey {
    /// A commit key.
    pub fn commit(hash: impl Into<String>) -> Self {
        TreeKey::Commit(hash.into())
    }

    /// The database key: `commit:<hash>` or `worktree:<hash>`.
    pub fn db_key(&self) -> String {
        match self {
            TreeKey::Commit(h) => format!("commit:{h}"),
            TreeKey::Worktree(h) => format!("worktree:{h}"),
        }
    }

    /// Parse a database key written by [`TreeKey::db_key`].
    pub fn from_db_key(key: &str) -> Option<Self> {
        let (kind, hash) = key.split_once(':')?;
        match kind {
            "commit" => Some(TreeKey::Commit(hash.to_string())),
            "worktree" => Some(TreeKey::Worktree(ContentHash(hash.to_string()))),
            _ => None,
        }
    }

    /// `commit` or `worktree`.
    pub fn kind(&self) -> &'static str {
        match self {
            TreeKey::Commit(_) => "commit",
            TreeKey::Worktree(_) => "worktree",
        }
    }
}

impl fmt::Display for TreeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.db_key())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_state_hash_is_order_independent() {
        let a = (Path::new("src/a.rs"), ContentHash::of_str("a"));
        let b = (Path::new("src/b.rs"), ContentHash::of_str("b"));
        let h1 = ContentHash::of_worktree_state([(a.0, &a.1), (b.0, &b.1)]);
        let h2 = ContentHash::of_worktree_state([(b.0, &b.1), (a.0, &a.1)]);
        assert_eq!(h1, h2);
        let c = (Path::new("src/b.rs"), ContentHash::of_str("c"));
        assert_ne!(
            h1,
            ContentHash::of_worktree_state([(a.0, &a.1), (c.0, &c.1)])
        );
    }

    #[test]
    fn a_facts_key_names_path_content_and_package() {
        let h = ContentHash::of_str("fn main() {}");
        let k = FactsKey::of(Path::new("src/main.rs"), &h, "p1");
        assert_eq!(k, FactsKey::of(Path::new("src/main.rs"), &h, "p1"));
        assert_ne!(k, FactsKey::of(Path::new("src/lib.rs"), &h, "p1"));
        assert_ne!(k, FactsKey::of(Path::new("src/main.rs"), &h, "p2"));
        let h2 = ContentHash::of_str("fn main() { }");
        assert_ne!(k, FactsKey::of(Path::new("src/main.rs"), &h2, "p1"));
    }

    #[test]
    fn tree_key_round_trips_through_db_key() {
        for k in [
            TreeKey::commit("abc"),
            TreeKey::Worktree(ContentHash::of_str("x")),
        ] {
            assert_eq!(TreeKey::from_db_key(&k.db_key()), Some(k));
        }
        assert_eq!(TreeKey::from_db_key("nope"), None);
    }
}
