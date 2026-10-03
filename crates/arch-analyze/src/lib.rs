//! `arch-analyze` · watcher · cargo · git · rust-analyzer → fact deltas.
//!
//! Depends on [`arch_facts`] only. Produces [`arch_facts::FileFacts`] per file (ADR 0002) for the
//! one unit of a repository (ADR 0007) and assembles them through the store.
//!
//! Two depths ([`Depth`]). The **syntax-level pass** reads each file's items from its syntax
//! tree and guesses links from the names the source spells out: every link is `guessed`, every
//! analyzed crate is listed in `Analyzer.degraded`, and findings on these facts warn and never
//! block (ADR 0009). The **resolved pass** asks rust-analyzer for the type-checked view and
//! marks what it confirms `resolved`; when rust-analyzer cannot load the repository the result
//! is the syntax-level one with the reason recorded (ADR 0010).
//!
//! ```no_run
//! let facts = arch_analyze::analyze(std::path::Path::new("."), &arch_analyze::Options::default())?;
//! println!("{} items, {} links", facts.items.len(), facts.links.len());
//! # Ok::<(), arch_analyze::Error>(())
//! ```

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use arch_facts::{
    Analyzer as Analyzer_, ArchDir, ContentHash, Degraded, Event, Facts, FactsKey, FileFacts, Repo,
    Store, TreeFile, TreeHead, TreeKey, Unit,
};

pub mod assemble;
pub mod cargo;
pub mod decl;
pub mod git;
pub mod items;
pub mod package;
pub mod pass;
pub mod ra;
pub mod sql;
pub mod watch;

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
    /// The file-system watcher failed.
    #[error("watcher: {0}")]
    Watch(String),
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
    /// Type-checked through rust-analyzer: links are `resolved`, pattern links stay `guessed`.
    /// When rust-analyzer cannot load the repository the result is the syntax-level one, with
    /// the reason recorded (ADR 0010).
    Resolved,
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
    /// Where cargo's build artifacts for the repository are, when not in its own `target/`
    /// (rust-analyzer reads build-script outputs and proc-macro libraries from there).
    pub target_dir: Option<PathBuf>,
}

/// Analyze the repository at `repo` and return its facts.
pub fn analyze(repo: &Path, options: &Options) -> Result<Facts, Error> {
    let mut store = Store::in_memory()?;
    let key = index(repo, options, &mut store)?;
    Ok(store.facts(&key)?.expect("the tree was just recorded"))
}

/// Analyze the repository at `repo` into `store` and return the key its facts are under.
pub fn index(repo: &Path, options: &Options, store: &mut Store) -> Result<TreeKey, Error> {
    Analyzer::open(repo, options.clone())?.index(store)
}

/// How an [`Analyzer::index`] recomputed the facts (ADR 0002).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recompute {
    /// The whole unit was analysed: the first index, or a change to a declaration, a manifest,
    /// or the set of Rust files.
    Unit,
    /// Only these files were analysed: the save changed function bodies, or files that hold no
    /// Rust. Every other file's delta was carried forward under its new key.
    Files(Vec<PathBuf>),
}

/// What the last index saw, for the next one to recompute only what a save changed.
#[derive(Debug)]
struct Last {
    /// Every file: its content hash and the key its facts are under.
    files: HashMap<PathBuf, (ContentHash, FactsKey)>,
    /// The declaration fingerprint of every Rust file the pass walked (`decl`).
    decls: HashMap<String, String>,
}

/// An analyzer kept open on one repository. With [`Depth::Resolved`] it holds rust-analyzer
/// loaded, so after [`Analyzer::file_changed`] the next [`Analyzer::index`] is incremental
/// (spec §5: one-file recompute < 500 ms). It also remembers what it last indexed: a save that
/// changes function bodies only re-analyses the changed files (ADR 0002).
#[derive(Debug)]
pub struct Analyzer {
    root: PathBuf,
    options: Options,
    session: Option<ra::Session>,
    load_error: Option<String>,
    /// How the repository names its objects, for package ids (ADR 0002).
    format: package::ObjectFormat,
    /// The unit plan, with what it was read from: manifests, lock file, `main_bin`.
    plan: Option<(PlanInputs, cargo::UnitPlan)>,
    last: Option<Last>,
    recompute: Option<Recompute>,
}

/// What `cargo metadata` reads: every manifest, the lock file, cargo's configuration, and the
/// `areas.toml` choice of main binary.
type PlanInputs = (Option<String>, Vec<(PathBuf, ContentHash)>);

fn is_plan_input(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n == "Cargo.toml" || n == "Cargo.lock")
        || path.starts_with(".cargo")
}

impl Analyzer {
    /// Open the repository at `repo`. With [`Depth::Resolved`] this loads rust-analyzer, which
    /// is where the first-index time goes; a load failure is kept as the degrade reason.
    pub fn open(repo: &Path, options: Options) -> Result<Self, Error> {
        let root = repo.canonicalize().map_err(|source| Error::Io {
            path: repo.to_path_buf(),
            source,
        })?;
        let (session, load_error) = match options.depth {
            Depth::Syntax => (None, None),
            Depth::Resolved => {
                match big_stack(|| ra::Session::load(&root, options.target_dir.as_deref())) {
                    Ok(s) => (Some(s), None),
                    Err(e) => (None, Some(format!("{e:#}"))),
                }
            }
        };
        let format = if git::is_repo(&root) {
            package::ObjectFormat::of_repo(&root)
        } else {
            package::ObjectFormat::default()
        };
        Ok(Analyzer {
            root,
            options,
            session,
            load_error,
            format,
            plan: None,
            last: None,
            recompute: None,
        })
    }

    /// How the last [`Analyzer::index`] recomputed the facts.
    pub fn last_recompute(&self) -> Option<&Recompute> {
        self.recompute.as_ref()
    }

    /// Why the facts are syntax-level only, when they are.
    pub fn degraded(&self) -> Option<String> {
        match (&self.session, &self.load_error) {
            (Some(_), _) => None,
            (None, Some(e)) => Some(format!("rust-analyzer could not load the repository: {e}")),
            (None, None) => Some(SYNTAX_REASON.to_string()),
        }
    }

    /// Tell the analyzer a file changed on disk (repository-relative path), or is gone.
    pub fn file_changed(&mut self, rel: &Path) -> Result<(), Error> {
        let path = self.root.join(rel);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => return Err(Error::Io { path, source }),
        };
        if let Some(s) = &mut self.session {
            s.file_changed(&rel.to_string_lossy().replace('\\', "/"), text);
        }
        Ok(())
    }

    /// Apply a watcher batch of this analyzer's worktree: tell it each changed file, index into
    /// `store`, and return the events: one `FilesChanged` per writer, then `FactsUpdated` naming
    /// the changed files.
    pub fn apply(&mut self, batch: &watch::Batch, store: &mut Store) -> Result<Vec<Event>, Error> {
        if batch.worktree != self.root {
            return Err(Error::Unit {
                path: self.root.clone(),
                reason: format!("a batch of another worktree: {}", batch.worktree.display()),
            });
        }
        for c in &batch.changes {
            self.file_changed(&c.path)?;
        }
        let tree = self.index(store)?;
        let mut events = batch.files_changed();
        events.push(Event::FactsUpdated {
            tree,
            files: batch.changes.iter().map(|c| c.path.clone()).collect(),
        });
        Ok(events)
    }

    /// Analyze into `store`: `put_tree` with every file's delta under its facts key (ADR 0002),
    /// then `set_worktree`. Returns the key; `store.facts(key)` assembles the facts.
    ///
    /// Against what the analyzer last indexed: when the Rust files that changed changed function
    /// bodies only (their declaration fingerprint is the same), only the changed files are
    /// analysed and every other file's delta is carried forward under its new key; otherwise the
    /// whole unit is. Either way the facts are what a cold analysis gives
    /// (`tests/incremental.rs`). [`Analyzer::last_recompute`] says which it was.
    pub fn index(&mut self, store: &mut Store) -> Result<TreeKey, Error> {
        let root = self.root.clone();
        let options = &self.options;
        let unit_err = |reason: String| Error::Unit {
            path: root.clone(),
            reason,
        };
        let in_git = git::is_repo(&root);

        let files = if in_git {
            git::files(&root).map_err(|e| Error::Git(format!("{e:#}")))?
        } else {
            walk_dir(&root)
        };
        let mut hashed: Vec<(PathBuf, ContentHash)> = Vec::with_capacity(files.len());
        let mut blobs: Vec<(PathBuf, package::Blob)> = Vec::with_capacity(files.len());
        let mut texts: HashMap<PathBuf, String> = HashMap::new();
        for f in &files {
            let path = root.join(f);
            let io = |source| Error::Io {
                path: path.clone(),
                source,
            };
            let bytes = std::fs::read(&path).map_err(io)?;
            hashed.push((f.clone(), ContentHash::of_bytes(&bytes)));
            if f.extension().is_some_and(|e| e == "rs")
                && let Ok(text) = std::str::from_utf8(&bytes)
            {
                texts.insert(f.clone(), text.to_string());
            }
            blobs.push((
                f.clone(),
                package::Blob::of_file(self.format, &path, &bytes).map_err(io)?,
            ));
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
        let inputs: PlanInputs = (
            main_bin.clone(),
            hashed
                .iter()
                .filter(|(p, _)| is_plan_input(p))
                .cloned()
                .collect(),
        );
        let plan_changed = self.plan.as_ref().is_none_or(|(i, _)| *i != inputs);
        if plan_changed {
            let plan =
                cargo::read(&root, main_bin.as_deref()).map_err(|e| unit_err(format!("{e:#}")))?;
            self.plan = Some((inputs, plan));
        }
        let plan = self.plan.as_ref().expect("read above").1.clone();
        let only = if plan_changed {
            None
        } else {
            self.changed_bodies(&hashed, &texts, store)?
        };

        let name = options
            .repo_name
            .clone()
            .or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "repo".into());
        let scope = format!("unit:{name}");
        // The database may move to another thread but not be shared with one: lend it mutably.
        let session = self.session.as_mut();
        let (r, p, sc, fs, o) = (&root, &plan, &scope, &files, only.as_ref());
        let out = big_stack(move || pass::run(r, p, sc, fs, session.as_deref(), o))
            .map_err(|e| unit_err(format!("{e:#}")))?;

        let reason = self.degraded().map(|r| match &plan.cargo_error {
            Some(e) => format!("{r}; {e}"),
            None => r,
        });
        let crates = plan.crates();
        let degraded = match &reason {
            Some(reason) => crates
                .iter()
                .filter(|c| c.status == arch_facts::CrateStatus::Analyzed)
                .map(|c| Degraded {
                    crate_name: c.name.clone(),
                    reason: reason.clone(),
                })
                .collect(),
            None => out
                .unresolved
                .iter()
                .map(|(krate, files)| Degraded {
                    crate_name: krate.clone(),
                    reason: unresolved_reason(files),
                })
                .collect(),
        };
        let version = format!("{}+ra_ap_{}", env!("CARGO_PKG_VERSION"), ra::RA_VERSION);
        let mut tree_head = TreeHead {
            repo: Repo {
                name,
                commit,
                unit: Unit {
                    id: scope,
                    main_target: plan.main_target.clone(),
                },
            },
            analyzer: Analyzer_ {
                name: "arch-analyze".into(),
                version,
                degraded,
            },
            crates,
            assembled: Default::default(),
        };

        let commits = match (&options.commits, in_git) {
            (Commits::None, _) | (_, false) => Vec::new(),
            (Commits::Range(r), true) => {
                git::commits(&root, r).map_err(|e| Error::Git(format!("{e:#}")))?
            }
            (Commits::Branch, true) => branch_commits(&root),
        };
        let keys = self.facts_keys(&plan, &out, &hashed, &blobs, &tree_head);
        let mut deltas = Vec::with_capacity(hashed.len());
        for (path, hash) in &hashed {
            let rel = path.to_string_lossy().replace('\\', "/");
            if let Some(only) = &only
                && !only.contains(&rel)
            {
                // Unchanged: its delta is carried forward (checked present by `changed_bodies`).
                let last = self.last.as_ref().expect("a last index to compare with");
                let (_, old) = &last.files[path];
                deltas.push(store.file_facts(old)?.expect("checked present"));
                continue;
            }
            let mut facts = FileFacts::empty(path.clone(), hash.clone());
            if let Some(parts) = out.files.get(&rel) {
                facts.items = parts.items.clone();
                facts.links = parts.links.clone();
                facts.notes = assemble::to_value(&parts.notes);
            }
            if out.rust_files.contains(&rel) {
                facts.degraded = reason.clone().or_else(|| {
                    let unresolved = out.unresolved.values().flatten().any(|f| *f == rel);
                    unresolved.then(|| unresolved_reason(std::slice::from_ref(&rel)))
                });
            }
            deltas.push(facts);
        }
        let lock = assemble::lock_packages(&root);
        let type_checked = self.session.is_some();
        tree_head.assembled = assemble::derive(&plan, &lock, &deltas, &out.walk, type_checked);

        let hashes: Vec<String> = commits.iter().map(|c| c.hash.clone()).collect();
        let tree_files: Vec<TreeFile> = hashed
            .iter()
            .zip(keys.iter().cloned())
            .map(|((path, file_hash), facts_key)| TreeFile {
                path: path.clone(),
                file_hash: file_hash.clone(),
                facts_key,
            })
            .collect();
        store.put_commits(&commits)?;
        store.put_tree(
            &key,
            head.as_deref(),
            &tree_files,
            &tree_head,
            &hashes,
            &deltas,
        )?;
        // This worktree is now on `key`: the state it leaves is forgotten unless another
        // worktree is on it (ADR 0002).
        let branch = in_git.then(|| git::current_branch(&root)).flatten();
        store.set_worktree(&root, &key, head.as_deref(), branch.as_deref())?;

        // Remember what was indexed, for the next save.
        let mut decls = self.last.take().map(|l| l.decls).unwrap_or_default();
        if only.is_none() {
            decls.clear();
        }
        for rel in &out.rust_files {
            if only.as_ref().is_none_or(|o| o.contains(rel))
                && let Some(text) = texts.get(Path::new(rel))
            {
                decls.insert(rel.clone(), decl::fingerprint(text));
            }
        }
        self.last = Some(Last {
            files: hashed
                .iter()
                .zip(keys)
                .map(|((p, h), k)| (p.clone(), (h.clone(), k)))
                .collect(),
            decls,
        });
        self.recompute = Some(match only {
            None => Recompute::Unit,
            Some(o) => Recompute::Files(o.into_iter().map(PathBuf::from).collect()),
        });
        Ok(key)
    }

    /// The files to analyse when the save since the last index changed function bodies only,
    /// or files that hold no Rust; `None` when the whole unit must be: no last index, a Rust
    /// file added or removed, a declaration changed, or a delta to carry is gone from `store`.
    fn changed_bodies(
        &self,
        hashed: &[(PathBuf, ContentHash)],
        texts: &HashMap<PathBuf, String>,
        store: &Store,
    ) -> Result<Option<BTreeSet<String>>, Error> {
        let Some(last) = &self.last else {
            return Ok(None);
        };
        let is_rs = |p: &Path| p.extension().is_some_and(|e| e == "rs");
        let now: HashMap<&Path, &ContentHash> =
            hashed.iter().map(|(p, h)| (p.as_path(), h)).collect();
        if last
            .files
            .keys()
            .any(|p| is_rs(p) && !now.contains_key(p.as_path()))
        {
            return Ok(None);
        }
        let mut only = BTreeSet::new();
        for (path, hash) in hashed {
            let rel = path.to_string_lossy().replace('\\', "/");
            match last.files.get(path) {
                None if is_rs(path) => return Ok(None),
                None => {
                    only.insert(rel);
                }
                Some((old, _)) if old != hash => {
                    if let Some(before) = last.decls.get(&rel) {
                        let Some(text) = texts.get(path) else {
                            return Ok(None);
                        };
                        if &decl::fingerprint(text) != before {
                            return Ok(None);
                        }
                    }
                    only.insert(rel);
                }
                Some((_, key)) => {
                    if !store.has_file_facts(key)? {
                        return Ok(None);
                    }
                }
            }
        }
        Ok(Some(only))
    }

    /// The facts key of each file (ADR 0002): its path, its content hash and the id of its
    /// package, or of the packages whose crates walk it. A file no package holds is named by the
    /// unit's id.
    fn facts_keys(
        &self,
        plan: &cargo::UnitPlan,
        out: &pass::Output,
        hashed: &[(PathBuf, ContentHash)],
        blobs: &[(PathBuf, package::Blob)],
        head: &TreeHead,
    ) -> Vec<FactsKey> {
        let trees = package::tree_ids(self.format, blobs);
        let lock = blobs
            .iter()
            .find(|(p, _)| p == Path::new("Cargo.lock"))
            .map(|(_, b)| b.hex());
        let analyzer = format!(
            "analyzer {} {}
unit {} {}
degraded {}
",
            head.analyzer.version,
            if self.session.is_some() {
                "resolved"
            } else {
                "syntax"
            },
            head.repo.unit.id,
            head.repo.unit.main_target,
            serde_json::to_string(&head.analyzer.degraded).unwrap_or_default(),
        );
        let ids = package::package_ids(plan, &trees, lock.as_deref(), &analyzer);
        // Innermost package directory first.
        let mut dirs: Vec<(PathBuf, usize)> = (0..plan.packages.len())
            .map(|p| (package::package_dir(plan, p), p))
            .collect();
        dirs.sort_by_key(|(d, _)| std::cmp::Reverse(d.components().count()));
        hashed
            .iter()
            .map(|(path, hash)| {
                let rel = path.to_string_lossy().replace('\\', "/");
                let package = match out.owners.get(&rel) {
                    Some(owners) => owners
                        .iter()
                        .map(|p| ids.packages[*p].as_str())
                        .collect::<Vec<_>>()
                        .join(","),
                    None => dirs
                        .iter()
                        .find(|(d, _)| path.starts_with(d))
                        .map_or(ids.unit.clone(), |(_, p)| ids.packages[*p].clone()),
                };
                FactsKey::of(path, hash, &package)
            })
            .collect()
    }
}

/// Why a crate's facts are partly guessed while rust-analyzer is loaded: it has no module for
/// these files, typically because their crate was added after it loaded.
fn unresolved_reason(files: &[String]) -> String {
    format!(
        "rust-analyzer has no module for {}: links guessed until the analyzer is opened again",
        files.join(", ")
    )
}

/// Run `f` on a thread with a stack large enough for rust-analyzer's type inference, which
/// recurses deeply; a caller's thread (a test's, a tokio worker's) is often too small.
fn big_stack<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn_scoped(scope, f)
            .expect("spawning the analysis thread")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
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
