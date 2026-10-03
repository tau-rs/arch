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
//! On Linux (inotify, one watch per directory) each worktree is watched directory by directory,
//! skipping the directories facts never come from, and a directory that appears is watched and
//! its files reported. When the platform dropped events (a queue overflow), the next batch of
//! each worktree concerned lists every file facts come from, so the facts catch up.
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
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};
use std::time::{Duration, Instant};

use arch_facts::{Attribution, ContentHash, Event};
use notify::event::{CreateKind, ModifyKind};
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
    let Some(parts) = parts(rel) else {
        return false;
    };
    let Some((name, dirs)) = parts.split_last() else {
        return false;
    };
    if !holds_sources(dirs) || name.starts_with('.') {
        return false;
    }
    *name == "Cargo.toml"
        || name.ends_with(".rs")
        || (name.ends_with(".sql") && dirs.contains(&"migrations"))
}

/// Whether files facts come from can be in this directory (relative to its worktree; the
/// worktree itself is `""`): not under `target/`, not hidden.
fn is_source_dir(rel: &Path) -> bool {
    parts(rel).is_some_and(|p| holds_sources(&p))
}

fn parts(rel: &Path) -> Option<Vec<&str>> {
    Some(rel.to_str()?.split(['/', '\\']).collect())
}

fn holds_sources(dirs: &[&str]) -> bool {
    dirs.first() != Some(&"target") && !dirs.iter().any(|p| p.starts_with('.'))
}

/// Whether worktrees are watched directory by directory. inotify (Linux) holds one watch per
/// directory, counted against `fs.inotify.max_user_watches`, and a built `target/` alone holds
/// thousands of directories: there, only the directories facts can come from are watched.
/// FSEvents (macOS) and Windows watch a whole tree with one handle, and each watch added restarts
/// the FSEvents stream: there, a worktree stays one recursive watch and the filter runs per event.
const PER_DIRECTORY: bool = cfg!(any(target_os = "linux", target_os = "android"));

/// Watches worktrees and hands out their debounced batches.
pub struct Watcher {
    /// Watched worktrees, longest first, so a worktree nested in another claims its own files.
    roots: Arc<RwLock<Vec<PathBuf>>>,
    watches: Arc<Mutex<Watches>>,
    batches: Receiver<Batch>,
    /// What the notify thread sends to the debounce thread; a test feeds events through it.
    #[cfg(test)]
    raw: Sender<Raw>,
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
        #[cfg(test)]
        let raw = raw_tx.clone();
        let seen_roots = Arc::clone(&roots);
        let notify = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            // An error carries no path to recompute; the next change to a file reaches the
            // facts as usual.
            let Ok(event) = res else { return };
            let roots = seen_roots.read().unwrap_or_else(PoisonError::into_inner);
            for raw in intake(&roots, event) {
                let _ = raw_tx.send(raw);
            }
        })
        .map_err(watch_error)?;
        let watches = Arc::new(Mutex::new(Watches {
            notify,
            dirs: BTreeSet::new(),
        }));
        // Weak: the debounce thread ends when the watcher, and so the notify handle, is dropped.
        let (loop_roots, loop_watches) = (Arc::clone(&roots), Arc::downgrade(&watches));
        std::thread::Builder::new()
            .name("arch-watch".into())
            .spawn(move || {
                debounce_loop(&raw_rx, &tx, debounce, |raw, burst| {
                    take(raw, &loop_roots, &loop_watches, burst);
                });
            })
            .map_err(|e| Error::Watch(format!("starting the debounce thread: {e}")))?;
        let mut watcher = Watcher {
            roots,
            watches,
            batches,
            #[cfg(test)]
            raw,
        };
        for w in worktrees {
            watcher.add(w)?;
        }
        Ok(watcher)
    }

    /// Start watching a worktree, e.g. one a session just created.
    pub fn add(&mut self, worktree: &Path) -> Result<(), Error> {
        let root = canonical(worktree)?;
        let roots = {
            let mut roots = self.roots.write().unwrap_or_else(PoisonError::into_inner);
            if !roots.contains(&root) {
                roots.push(root.clone());
                roots.sort_by_key(|r| std::cmp::Reverse(r.as_os_str().len()));
            }
            roots.clone()
        };
        let mut watches = self.watches.lock().unwrap_or_else(PoisonError::into_inner);
        let watched = if PER_DIRECTORY {
            // A worktree nested in a watched one: its `target/` was watched under the outer
            // worktree's rule, and is no longer.
            watches.prune(&roots, &root);
            scan(&roots, &root)
                .dirs
                .iter()
                .try_for_each(|d| watches.watch(d, RecursiveMode::NonRecursive))
        } else {
            watches.watch(&root, RecursiveMode::Recursive)
        };
        drop(watches);
        if let Err(e) = watched {
            // Watch all of it or none of it: half a worktree would miss changes silently.
            let _ = self.remove(&root);
            return Err(watch_error(e));
        }
        Ok(())
    }

    /// Stop watching a worktree, e.g. one a session archived.
    pub fn remove(&mut self, worktree: &Path) -> Result<(), Error> {
        let root = canonical(worktree)?;
        let roots = {
            let mut roots = self.roots.write().unwrap_or_else(PoisonError::into_inner);
            roots.retain(|r| r != &root);
            roots.clone()
        };
        let mut watches = self.watches.lock().unwrap_or_else(PoisonError::into_inner);
        if PER_DIRECTORY {
            watches.prune(&roots, &root);
            Ok(())
        } else {
            watches.dirs.remove(&root);
            watches.notify.unwatch(&root).map_err(watch_error)
        }
    }

    /// The next batch, waiting as long as it takes.
    pub fn recv(&self) -> Option<Batch> {
        self.batches.recv().ok()
    }

    /// The next batch, or `None` when none closed within `timeout`.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Batch> {
        self.batches.recv_timeout(timeout).ok()
    }

    /// Hand `event` to the watcher as if the platform had sent it (a test cannot make inotify
    /// overflow on demand).
    #[cfg(test)]
    fn feed(&self, event: notify::Event) {
        let roots = self.roots.read().unwrap_or_else(PoisonError::into_inner);
        for raw in intake(&roots, event) {
            self.raw.send(raw).unwrap();
        }
    }
}

/// The notify handle and the directories it watches. [`Watcher::add`] and [`Watcher::remove`]
/// use it, and so does the debounce thread, to watch directories as they appear. (The notify
/// thread cannot: on inotify, adding a watch waits on that same thread.)
struct Watches {
    notify: RecommendedWatcher,
    dirs: BTreeSet<PathBuf>,
}

impl Watches {
    fn watch(&mut self, dir: &Path, mode: RecursiveMode) -> notify::Result<()> {
        self.notify.watch(dir, mode)?;
        self.dirs.insert(dir.to_path_buf());
        Ok(())
    }

    /// Stop watching the directories under `under` no worktree of `roots` takes files from.
    fn prune(&mut self, roots: &[PathBuf], under: &Path) {
        let gone: Vec<PathBuf> = self
            .dirs
            .iter()
            .filter(|d| d.starts_with(under) && !takes_files(roots, d))
            .cloned()
            .collect();
        for d in gone {
            // The directory may be gone already, and its watch with it.
            let _ = self.notify.unwatch(&d);
            self.dirs.remove(&d);
        }
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

/// The innermost worktree a path is in, and the path relative to it.
fn within<'r>(roots: &'r [PathBuf], path: &'r Path) -> Option<(&'r PathBuf, &'r Path)> {
    let root = roots.iter().find(|r| path.starts_with(r))?;
    Some((root, path.strip_prefix(root).ok()?))
}

/// The worktree a path is in and the path relative to it, when facts come from it.
fn locate(roots: &[PathBuf], path: &Path) -> Option<(PathBuf, PathBuf)> {
    let (root, rel) = within(roots, path)?;
    is_watched(rel).then(|| (root.clone(), rel.to_path_buf()))
}

/// Whether a directory is one its innermost worktree takes files from.
fn takes_files(roots: &[PathBuf], dir: &Path) -> bool {
    within(roots, dir).is_some_and(|(_, rel)| is_source_dir(rel))
}

/// What the notify thread hands to the debounce thread.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Raw {
    /// A file facts come from changed: its worktree, and its path relative to it.
    File(PathBuf, PathBuf),
    /// Something appeared at this path, in a place facts can come from. When it is a directory,
    /// it is watched and the files already in it are reported: a file written before its
    /// directory's watch landed raised no event.
    Appeared(PathBuf),
    /// The platform dropped events (inotify's queue overflowed, FSEvents asks for a rescan). The
    /// worktrees holding these paths, or every worktree when there are none, may have changed
    /// anywhere.
    Rescan(Vec<PathBuf>),
}

/// What one notify event means for the burst.
fn intake(roots: &[PathBuf], event: notify::Event) -> Vec<Raw> {
    if event.need_rescan() {
        return vec![Raw::Rescan(event.paths)];
    }
    let appeared = match event.kind {
        EventKind::Access(_) => return Vec::new(),
        EventKind::Create(kind) => kind != CreateKind::File,
        EventKind::Modify(ModifyKind::Name(_)) => true,
        _ => false,
    };
    let mut raw = Vec::new();
    for path in &event.paths {
        let Some((root, rel)) = within(roots, path) else {
            continue;
        };
        if appeared && is_source_dir(rel) {
            raw.push(Raw::Appeared(path.clone()));
        }
        if is_watched(rel) {
            raw.push(Raw::File(root.clone(), rel.to_path_buf()));
        }
    }
    raw
}

/// The files of a burst, by worktree.
type Burst = BTreeMap<PathBuf, BTreeSet<PathBuf>>;

/// Add what `raw` names to the burst, watching the directories that appeared.
fn take(raw: Raw, roots: &RwLock<Vec<PathBuf>>, watches: &Weak<Mutex<Watches>>, burst: &mut Burst) {
    let roots = roots.read().unwrap_or_else(PoisonError::into_inner).clone();
    let starts = match raw {
        Raw::File(root, rel) => {
            burst.entry(root).or_default().insert(rel);
            return;
        }
        Raw::Appeared(path) => vec![path],
        Raw::Rescan(paths) if paths.is_empty() => roots.clone(),
        Raw::Rescan(paths) => paths
            .iter()
            .filter_map(|p| within(&roots, p).map(|(root, _)| root.clone()))
            .collect(),
    };
    for start in starts {
        let found = scan(&roots, &start);
        if let Some(watches) = watches.upgrade().filter(|_| PER_DIRECTORY) {
            let mut watches = watches.lock().unwrap_or_else(PoisonError::into_inner);
            for d in &found.dirs {
                // Watched again even when known: a directory removed and created again lost
                // its watch. One that is gone again needs none.
                let _ = watches.watch(d, RecursiveMode::NonRecursive);
            }
        }
        for (root, rel) in found.files {
            burst.entry(root).or_default().insert(rel);
        }
    }
}

/// The directories at and under a path that files facts come from can be in, and those files.
#[derive(Debug, Default)]
struct Scan {
    dirs: Vec<PathBuf>,
    files: Vec<(PathBuf, PathBuf)>,
}

/// Walk `start`. Each directory is judged against its innermost worktree, so a worktree nested
/// in another is walked under its own rule. Symbolic links to directories are not followed.
fn scan(roots: &[PathBuf], start: &Path) -> Scan {
    let mut found = Scan::default();
    let mut todo = vec![start.to_path_buf()];
    while let Some(dir) = todo.pop() {
        if !takes_files(roots, &dir) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                todo.push(path);
            } else if let Some(file) = locate(roots, &path) {
                found.files.push(file);
            }
        }
        found.dirs.push(dir);
    }
    found
}

/// Gather paths into a burst until it is quiet or old enough, then send one batch per worktree.
/// Ends when the watcher is dropped.
fn debounce_loop(
    raw: &Receiver<Raw>,
    out: &Sender<Batch>,
    d: Debounce,
    mut take: impl FnMut(Raw, &mut Burst),
) {
    while let Ok(first) = raw.recv() {
        let start = Instant::now();
        let mut last = start;
        let mut burst = Burst::new();
        take(first, &mut burst);
        let mut open = true;
        loop {
            let deadline = (last + d.quiet).min(start + d.max_wait);
            let Some(wait) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            match raw.recv_timeout(wait) {
                Ok(found) => {
                    take(found, &mut burst);
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

    fn write(root: &Path, path: &str) {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "// x\n").unwrap();
    }

    /// A worktree with sources, build output, hidden directories, and a second worktree nested
    /// in a hidden directory of it (`<r>/.w/w1`), as a session's worktree may be.
    fn tree() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path().canonicalize().unwrap();
        for p in [
            "Cargo.toml",
            "README.md",
            "src/lib.rs",
            "src/a.rs",
            "target/debug/deps/x.rs",
            "target/debug/build/y/out/gen.rs",
            ".git/objects/ab/cd",
            ".w/w1/Cargo.toml",
            ".w/w1/src/lib.rs",
            ".w/w1/target/debug/z.rs",
        ] {
            write(&r, p);
        }
        let w1 = r.join(".w/w1");
        (tmp, r, w1)
    }

    fn roots(of: &[&PathBuf]) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = of.iter().map(|r| (*r).clone()).collect();
        roots.sort_by_key(|r| std::cmp::Reverse(r.as_os_str().len()));
        roots
    }

    fn sorted<T: Ord>(mut v: Vec<T>) -> Vec<T> {
        v.sort();
        v
    }

    /// Drop what the platform reports about files written before the watch started.
    fn settle(w: &Watcher) {
        while w.recv_timeout(Duration::from_millis(300)).is_some() {}
    }

    #[test]
    fn a_walk_skips_build_output_and_hidden_directories_and_a_nested_worktree_keeps_its_rule() {
        let (_tmp, r, w1) = tree();
        let roots = roots(&[&r, &w1]);
        let outer = scan(&roots, &r);
        assert_eq!(sorted(outer.dirs), [r.clone(), r.join("src")]);
        assert_eq!(
            sorted(outer.files),
            [
                (r.clone(), "Cargo.toml".into()),
                (r.clone(), "src/a.rs".into()),
                (r.clone(), "src/lib.rs".into()),
            ]
        );
        let inner = scan(&roots, &w1);
        assert_eq!(sorted(inner.dirs), [w1.clone(), w1.join("src")]);
        assert_eq!(
            scan(&roots, &r.join("target/debug")).dirs,
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn events_under_build_output_never_reach_the_burst() {
        let (_tmp, r, w1) = tree();
        let roots = roots(&[&r, &w1]);
        let ev = |kind: EventKind, path: PathBuf| notify::Event::new(kind).add_path(path);
        let folder = EventKind::Create(CreateKind::Folder);
        let file = EventKind::Create(CreateKind::File);
        for event in [
            ev(folder, r.join("target")),
            ev(folder, r.join("target/debug/incremental")),
            ev(file, r.join("target/debug/build/y/out/gen.rs")),
            ev(folder, w1.join("target")),
            ev(folder, r.join(".git/objects/ef")),
            ev(
                EventKind::Access(notify::event::AccessKind::Any),
                r.join("src/lib.rs"),
            ),
        ] {
            assert_eq!(intake(&roots, event.clone()), [], "{event:?}");
        }
        assert_eq!(
            intake(&roots, ev(folder, w1.join("src/x"))),
            [Raw::Appeared(w1.join("src/x"))]
        );
        assert_eq!(
            intake(&roots, ev(file, w1.join("src/x.rs"))),
            [Raw::File(w1.clone(), "src/x.rs".into())]
        );
        let rescan = notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan);
        assert_eq!(intake(&roots, rescan), [Raw::Rescan(Vec::new())]);
    }

    #[test]
    fn nothing_under_build_output_or_hidden_directories_is_watched() {
        let (_tmp, r, w1) = tree();
        let watcher = Watcher::new(&[r.clone(), w1.clone()], Debounce::default()).unwrap();
        settle(&watcher);

        // A build after the watch started, and a new source directory.
        std::fs::create_dir_all(r.join("target/release/deps/a")).unwrap();
        std::fs::create_dir_all(w1.join("target/release/deps/b")).unwrap();
        write(&r, "src/new/mod.rs");
        let batch = watcher
            .recv_timeout(Duration::from_secs(10))
            .expect("a batch");
        assert_eq!(batch.changes[0].path, Path::new("src/new/mod.rs"));
        settle(&watcher);

        let dirs: Vec<PathBuf> = {
            let watches = watcher.watches.lock().unwrap();
            watches.dirs.iter().cloned().collect()
        };
        let expected = if PER_DIRECTORY {
            sorted(vec![
                r.clone(),
                r.join("src"),
                r.join("src/new"),
                w1.clone(),
                w1.join("src"),
            ])
        } else {
            sorted(vec![r.clone(), w1.clone()])
        };
        assert_eq!(dirs, expected);
    }

    #[test]
    fn a_rescan_notice_gives_one_whole_worktree_batch() {
        let (_tmp, r, w1) = tree();
        let watcher = Watcher::new(&[r.clone(), w1.clone()], Debounce::default()).unwrap();
        settle(&watcher);
        let rescan = || notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan);

        // Dropped events somewhere in the nested worktree: that worktree, every file.
        watcher.feed(rescan().add_path(w1.join("src")));
        let batch = watcher
            .recv_timeout(Duration::from_secs(10))
            .expect("a batch");
        assert_eq!(batch.worktree, w1);
        let paths: Vec<&Path> = batch.changes.iter().map(|c| c.path.as_path()).collect();
        assert_eq!(paths, [Path::new("Cargo.toml"), Path::new("src/lib.rs")]);
        assert!(
            batch
                .changes
                .iter()
                .all(|c| !c.removed && c.attribution == Attribution::You)
        );
        assert_eq!(watcher.recv_timeout(Duration::from_millis(500)), None);

        // Dropped events with no path: every worktree, one batch each.
        watcher.feed(rescan());
        let mut batches: Vec<(PathBuf, usize)> = (0..2)
            .map(|_| {
                watcher
                    .recv_timeout(Duration::from_secs(10))
                    .expect("a batch")
            })
            .map(|b| (b.worktree, b.changes.len()))
            .collect();
        batches.sort();
        assert_eq!(batches, [(r, 3), (w1, 2)]);
        assert_eq!(watcher.recv_timeout(Duration::from_millis(500)), None);
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
