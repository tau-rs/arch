//! The watcher (spec §6 Sync; ADR 0002, 0011): one watcher over the repository and every
//! recorded worktree. Any change, whether yours, a pull, or another editor's, enters the same
//! path.
//!
//! A burst of file-system events (a save, a `git checkout`, a pull) becomes one [`Batch`] per
//! worktree once the worktree has been quiet for [`Debounce::quiet`]. Only the files facts come
//! from are reported ([`is_watched`]). Each changed file carries who wrote it ([`attribute`]):
//! a write arch's tool layer expected is the session's, so it does not come back to the session
//! as news; anything else is `you`.
//!
//! ```no_run
//! # use std::path::PathBuf;
//! # use arch_analyze::{Analyzer, Options, watch::{Debounce, Watcher}};
//! let repo = PathBuf::from(".");
//! let mut analyzer = Analyzer::open(&repo, Options::default())?;
//! let mut store = arch_facts::Store::in_memory()?;
//! let watcher = Watcher::new(&[repo], Debounce::default())?;
//! while let Some(batch) = watcher.recv() {
//!     for event in analyzer.apply(&batch, &mut store)? {
//!         println!("{event:?}");
//!     }
//! }
//! # Ok::<(), arch_analyze::Error>(())
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};

use arch_facts::{Attribution, ContentHash, Event};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::Error;

mod tag;

pub use tag::attribute;

/// How bursts are cut into batches.
///
/// The one-file budget (< 500 ms, ADR 0026) is measured from the save, so `quiet` counts against
/// it: it is kept just long enough to gather an editor's write-and-rename or one checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Debounce {
    /// A worktree's batch closes once no watched file changed in it for this long.
    pub quiet: Duration,
    /// A batch closes after this long even while changes keep coming, so a long stream of
    /// writes still reaches the facts.
    pub max_wait: Duration,
}

impl Default for Debounce {
    fn default() -> Self {
        Debounce {
            quiet: Duration::from_millis(50),
            max_wait: Duration::from_secs(2),
        }
    }
}

/// One changed file of a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The file, relative to the worktree.
    pub path: PathBuf,
    /// The file is gone.
    pub removed: bool,
    /// Who wrote it. A removed file is always `you`: the tool layer does not delete.
    pub attribution: Attribution,
}

/// The changes of one worktree over one burst.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    /// The worktree (canonical path).
    pub worktree: PathBuf,
    /// The changed files, sorted by path.
    pub changes: Vec<Change>,
}

impl Batch {
    /// One [`Event::FilesChanged`] per writer, in the order writers first appear.
    pub fn files_changed(&self) -> Vec<Event> {
        let mut by: Vec<(&Attribution, Vec<PathBuf>)> = Vec::new();
        for c in &self.changes {
            match by.iter_mut().find(|(a, _)| *a == &c.attribution) {
                Some((_, files)) => files.push(c.path.clone()),
                None => by.push((&c.attribution, vec![c.path.clone()])),
            }
        }
        by.into_iter()
            .map(|(attribution, files)| Event::FilesChanged {
                worktree: self.worktree.clone(),
                files,
                attribution: attribution.clone(),
            })
            .collect()
    }
}

/// Whether facts come from this file (relative to its worktree): Rust sources, manifests and
/// migrations (`migrations/*.sql`, at any depth). Build output (`target/`) and hidden files and
/// directories (`.git/`, `.arch/`, an editor's `.#lock`) never are.
pub fn is_watched(rel: &Path) -> bool {
    let Some(rel) = rel.to_str() else {
        return false;
    };
    let parts: Vec<&str> = rel.split(['/', '\\']).collect();
    let Some((name, dirs)) = parts.split_last() else {
        return false;
    };
    if dirs.first() == Some(&"target") || parts.iter().any(|p| p.starts_with('.')) {
        return false;
    }
    *name == "Cargo.toml"
        || name.ends_with(".rs")
        || (name.ends_with(".sql") && dirs.contains(&"migrations"))
}

/// Watches worktrees and hands out their debounced batches.
pub struct Watcher {
    notify: RecommendedWatcher,
    /// Watched worktrees, longest first, so a worktree nested in another claims its own files.
    roots: Arc<RwLock<Vec<PathBuf>>>,
    batches: Receiver<Batch>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher")
            .field("roots", &self.roots)
            .finish()
    }
}

impl Watcher {
    /// Watch `worktrees` (the repository and every recorded worktree, ADR 0011).
    pub fn new(worktrees: &[PathBuf], debounce: Debounce) -> Result<Self, Error> {
        let roots: Arc<RwLock<Vec<PathBuf>>> = Arc::default();
        let (raw_tx, raw_rx) = mpsc::channel();
        let (tx, batches) = mpsc::channel();
        let seen_roots = Arc::clone(&roots);
        let notify = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            // An error or a "rescan" notice carries no path to recompute; the next change to a
            // file reaches the facts as usual.
            let Ok(event) = res else { return };
            if matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            let roots = seen_roots.read().unwrap_or_else(PoisonError::into_inner);
            for path in event.paths {
                if let Some(found) = locate(&roots, &path) {
                    let _ = raw_tx.send(found);
                }
            }
        })
        .map_err(watch_error)?;
        std::thread::Builder::new()
            .name("arch-watch".into())
            .spawn(move || debounce_loop(&raw_rx, &tx, debounce))
            .map_err(|e| Error::Watch(format!("starting the debounce thread: {e}")))?;
        let mut watcher = Watcher {
            notify,
            roots,
            batches,
        };
        for w in worktrees {
            watcher.add(w)?;
        }
        Ok(watcher)
    }

    /// Start watching a worktree, e.g. one a session just created.
    pub fn add(&mut self, worktree: &Path) -> Result<(), Error> {
        let root = canonical(worktree)?;
        self.notify
            .watch(&root, RecursiveMode::Recursive)
            .map_err(watch_error)?;
        let mut roots = self.roots.write().unwrap_or_else(PoisonError::into_inner);
        if !roots.contains(&root) {
            roots.push(root);
            roots.sort_by_key(|r| std::cmp::Reverse(r.as_os_str().len()));
        }
        Ok(())
    }

    /// Stop watching a worktree, e.g. one a session archived.
    pub fn remove(&mut self, worktree: &Path) -> Result<(), Error> {
        let root = canonical(worktree)?;
        self.roots
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|r| r != &root);
        self.notify.unwatch(&root).map_err(watch_error)
    }

    /// The next batch, waiting as long as it takes.
    pub fn recv(&self) -> Option<Batch> {
        self.batches.recv().ok()
    }

    /// The next batch, or `None` when none closed within `timeout`.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Batch> {
        self.batches.recv_timeout(timeout).ok()
    }
}

fn canonical(path: &Path) -> Result<PathBuf, Error> {
    path.canonicalize().map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn watch_error(e: notify::Error) -> Error {
    Error::Watch(e.to_string())
}

/// The worktree a path is in and the path relative to it, when facts come from it.
fn locate(roots: &[PathBuf], path: &Path) -> Option<(PathBuf, PathBuf)> {
    let root = roots.iter().find(|r| path.starts_with(r))?;
    let rel = path.strip_prefix(root).ok()?;
    is_watched(rel).then(|| (root.clone(), rel.to_path_buf()))
}

/// Gather paths into a burst until it is quiet or old enough, then send one batch per worktree.
/// Ends when the watcher is dropped.
fn debounce_loop(raw: &Receiver<(PathBuf, PathBuf)>, out: &Sender<Batch>, d: Debounce) {
    while let Ok(first) = raw.recv() {
        let start = Instant::now();
        let mut last = start;
        let mut burst: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
        let mut add = |(root, rel)| {
            burst.entry(root).or_default().insert(rel);
        };
        add(first);
        let mut open = true;
        loop {
            let deadline = (last + d.quiet).min(start + d.max_wait);
            let Some(wait) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            match raw.recv_timeout(wait) {
                Ok(found) => {
                    add(found);
                    last = Instant::now();
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    open = false;
                    break;
                }
            }
        }
        for (worktree, files) in burst {
            let changes = files
                .into_iter()
                .map(|rel| change(&worktree, rel))
                .collect();
            if out.send(Batch { worktree, changes }).is_err() {
                return;
            }
        }
        if !open {
            return;
        }
    }
}

/// What happened to a file of a closed burst, read from the disk as it is now.
fn change(worktree: &Path, path: PathBuf) -> Change {
    match std::fs::read(worktree.join(&path)) {
        Ok(bytes) => {
            let attribution = attribute(worktree, &path, &ContentHash::of_bytes(&bytes));
            Change {
                path,
                removed: false,
                attribution,
            }
        }
        Err(e) => Change {
            path,
            removed: e.kind() == std::io::ErrorKind::NotFound,
            attribution: Attribution::You,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_come_from_sources_manifests_and_migrations_only() {
        for p in [
            "src/lib.rs",
            "Cargo.toml",
            "crates/a/Cargo.toml",
            "migrations/20240101_init.sql",
            "crates/db/migrations/1.sql",
            "src/target.rs",
            "crates/a/src/target/mod.rs",
        ] {
            assert!(is_watched(Path::new(p)), "{p}");
        }
        for p in [
            "README.md",
            "Cargo.lock",
            "schema.sql",
            "target/debug/build/x/out/gen.rs",
            ".git/HEAD",
            ".arch/areas.toml",
            ".arch/cache/x.rs",
            "src/.#lib.rs",
            "src/lib.rs~",
            "src/lib.rs.tmp.123",
        ] {
            assert!(!is_watched(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn a_path_belongs_to_the_innermost_worktree() {
        let mut roots = vec![PathBuf::from("/r"), PathBuf::from("/r/.w/w1")];
        roots.sort_by_key(|r| std::cmp::Reverse(r.as_os_str().len()));
        assert_eq!(
            locate(&roots, Path::new("/r/.w/w1/src/a.rs")),
            Some(("/r/.w/w1".into(), "src/a.rs".into()))
        );
        assert_eq!(
            locate(&roots, Path::new("/r/src/a.rs")),
            Some(("/r".into(), "src/a.rs".into()))
        );
        assert_eq!(locate(&roots, Path::new("/rr/src/a.rs")), None);
        assert_eq!(locate(&roots, Path::new("/r/target/a.rs")), None);
    }

    #[test]
    fn writers_are_grouped_in_the_order_they_first_appear() {
        let you = Attribution::You;
        let arch = Attribution::Arch;
        let c = |p: &str, a: &Attribution| Change {
            path: p.into(),
            removed: false,
            attribution: a.clone(),
        };
        let batch = Batch {
            worktree: "/r".into(),
            changes: vec![c("a.rs", &arch), c("b.rs", &you), c("c.rs", &arch)],
        };
        let files: Vec<(Vec<PathBuf>, Attribution)> = batch
            .files_changed()
            .into_iter()
            .map(|e| match e {
                Event::FilesChanged {
                    files, attribution, ..
                } => (files, attribution),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            files,
            [
                (vec!["a.rs".into(), "c.rs".into()], arch),
                (vec!["b.rs".into()], you),
            ]
        );
    }
}
