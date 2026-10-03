//! `arch-analyze` · watcher · cargo · git · rust-analyzer → fact deltas.
//!
//! Depends on [`arch_facts`] only. Produces [`arch_facts::FileFacts`] per file (ADR 0002) for the
//! one unit of a repository (ADR 0007) and assembles them through the store.
//!
//! What exists today is the **syntax-level pass**: cargo says what the unit is, each file's items
//! are read from its syntax tree, and links are guessed from the names the source spells out.
//! Every link is `guessed` and every analyzed crate is listed in `Analyzer.degraded`, so findings
//! on these facts warn and never block (ADR 0009). It is also the fallback for a crate
//! rust-analyzer cannot load (ADR 0010); the rust-analyzer pass (issue #3) refines it.
//!
//! ```no_run
//! let facts = arch_analyze::analyze(std::path::Path::new("."), &arch_analyze::Options::default())?;
//! println!("{} items, {} links", facts.items.len(), facts.links.len());
//! # Ok::<(), arch_analyze::Error>(())
//! ```

use std::path::{Path, PathBuf};

use arch_facts::{
    Analyzer, ArchDir, ContentHash, Degraded, Facts, FileFacts, Repo, Store, TreeHead, TreeKey,
    Unit,
};

pub mod cargo;
pub mod git;
pub mod items;
pub mod pass;
pub mod sql;

/// The reason recorded on every crate and file the syntax-level pass produced.
pub const SYNTAX_REASON: &str = "syntax-level pass: not type-checked";

/// Errors at the crate's boundary.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The path is not a cargo project the analyzer can read.
    #[error("{path}: {reason}")]
    Unit {
        /// The repository path.
        path: PathBuf,
        /// What went wrong, as the tool reported it.
        reason: String,
    },
    /// A file could not be read.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// Git answered with an error.
    #[error("git: {0}")]
    Git(String),
    /// The store or an `.arch/` file failed.
    #[error(transparent)]
    Facts(#[from] arch_facts::Error),
}

/// How deep the analysis goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Depth {
    /// Syntax only: fast, needs no build, every link `guessed`.
    #[default]
    Syntax,
}

/// Which commits become facts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Commits {
    /// The commits on the current branch since it left the default branch; none on the default
    /// branch itself or outside a repository.
    #[default]
    Branch,
    /// No commits.
    None,
    /// A git revision range, e.g. `main..HEAD`.
    Range(String),
}

/// Options for [`analyze`] and [`index`].
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Analysis depth.
    pub depth: Depth,
    /// Repository name in the facts; the directory name when absent.
    pub repo_name: Option<String>,
    /// Key the facts by this commit hash instead of asking git (a fixture that lives inside
    /// another repository names the commit its sources were last changed at).
    pub commit: Option<String>,
    /// Which commits to read.
    pub commits: Commits,
}

/// Analyze the repository at `repo` and return its facts.
pub fn analyze(repo: &Path, options: &Options) -> Result<Facts, Error> {
    let mut store = Store::in_memory()?;
    let key = index(repo, options, &mut store)?;
    Ok(store.facts(&key)?.expect("the tree was just recorded"))
}

/// Analyze the repository at `repo` into `store` and return the key its facts are under:
/// `put_tree → put_file_facts → facts(key)`.
///
/// The syntax-level pass recomputes every file of the unit on each call and overwrites their
/// deltas, because a file's links depend on other files (arch-design issue 21).
pub fn index(repo: &Path, options: &Options, store: &mut Store) -> Result<TreeKey, Error> {
    let unit_err = |reason: String| Error::Unit {
        path: repo.to_path_buf(),
        reason,
    };
    let root = repo.canonicalize().map_err(|source| Error::Io {
        path: repo.to_path_buf(),
        source,
    })?;
    let in_git = git::is_repo(&root);

    let files = if in_git {
        git::files(&root).map_err(|e| Error::Git(format!("{e:#}")))?
    } else {
        walk_dir(&root)
    };
    let mut hashed: Vec<(PathBuf, ContentHash)> = Vec::with_capacity(files.len());
    for f in &files {
        let bytes = std::fs::read(root.join(f)).map_err(|source| Error::Io {
            path: root.join(f),
            source,
        })?;
        hashed.push((f.clone(), ContentHash::of_bytes(&bytes)));
    }

    let clean = in_git && !git::is_dirty(&root).unwrap_or(true);
    let head = if in_git {
        git::rev_parse(&root, "HEAD").ok()
    } else {
        None
    };
    let key = match (&options.commit, &head) {
        (Some(c), _) => TreeKey::commit(c.clone()),
        (None, Some(h)) if clean => TreeKey::commit(h.clone()),
        _ => TreeKey::Worktree(ContentHash::of_worktree_state(
            hashed.iter().map(|(p, h)| (p.as_path(), h)),
        )),
    };
    let commit = match &key {
        TreeKey::Commit(h) => h.clone(),
        TreeKey::Worktree(h) => format!("wt:{h}"),
    };

    let arch = ArchDir::of_repo(&root);
    let main_bin = if arch.exists() {
        arch.read_areas().ok().and_then(|a| a.main_bin)
    } else {
        None
    };
    let plan = cargo::read(&root, main_bin.as_deref()).map_err(|e| unit_err(format!("{e:#}")))?;

    let name = options
        .repo_name
        .clone()
        .or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "repo".into());
    let scope = format!("unit:{name}");
    let out = pass::run(&root, &plan, &scope, &files).map_err(|e| unit_err(format!("{e:#}")))?;

    let reason = match &plan.cargo_error {
        Some(e) => format!("{SYNTAX_REASON}; {e}"),
        None => SYNTAX_REASON.to_string(),
    };
    let crates = plan.crates();
    let degraded = crates
        .iter()
        .filter(|c| c.status == arch_facts::CrateStatus::Analyzed)
        .map(|c| Degraded {
            crate_name: c.name.clone(),
            reason: reason.clone(),
        })
        .collect();
    let tree_head = TreeHead {
        repo: Repo {
            name,
            commit,
            unit: Unit {
                id: scope,
                main_target: plan.main_target.clone(),
            },
        },
        analyzer: Analyzer {
            name: "arch-analyze".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            degraded,
        },
        crates,
    };

    let commits = match (&options.commits, in_git) {
        (Commits::None, _) | (_, false) => Vec::new(),
        (Commits::Range(r), true) => {
            git::commits(&root, r).map_err(|e| Error::Git(format!("{e:#}")))?
        }
        (Commits::Branch, true) => branch_commits(&root),
    };
    let hashes: Vec<String> = commits.iter().map(|c| c.hash.clone()).collect();
    store.put_commits(&commits)?;
    store.put_tree(&key, head.as_deref(), &hashed, &tree_head, &hashes)?;

    for (path, hash) in &hashed {
        let rel = path.to_string_lossy().replace('\\', "/");
        let mut facts = FileFacts::empty(path.clone(), hash.clone());
        if let Some(parts) = out.files.get(&rel) {
            facts.items = parts.items.clone();
            facts.links = parts.links.clone();
            facts.ports = parts.ports.clone();
            facts.externals = parts.externals.clone();
            facts.entries = parts.entries.clone();
            facts.tables = parts.tables.clone();
        }
        if out.rust_files.contains(&rel) {
            facts.degraded = Some(reason.clone());
        }
        store.put_file_facts(&facts)?;
    }
    Ok(key)
}

/// Commits since the branch left the default branch; empty on the default branch.
fn branch_commits(root: &Path) -> Vec<arch_facts::Commit> {
    let Some(default) = git::default_branch(root) else {
        return Vec::new();
    };
    let Ok(base) = git::merge_base(root, &default, "HEAD") else {
        return Vec::new();
    };
    git::commits(root, &format!("{base}..HEAD")).unwrap_or_default()
}

/// Files under a directory that is not a git repository: everything but `target/` and
/// dot-directories other than `.arch/`.
fn walk_dir(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(root.join(&dir)) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let rel = dir.join(&name);
            match e.file_type() {
                Ok(t) if t.is_dir() => {
                    if name != "target" && (!name.starts_with('.') || name == ".arch") {
                        stack.push(rel);
                    }
                }
                Ok(t) if t.is_file() => out.push(rel),
                _ => {}
            }
        }
    }
    out.sort();
    out
}
