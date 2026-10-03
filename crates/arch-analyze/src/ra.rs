//! A loaded rust-analyzer: the type-checked view of the repository, kept in memory so one
//! changed file is recomputed without loading again (spec §5: one-file recompute < 500 ms).
//!
//! The `ra_ap_*` crates are rust-analyzer published as a library; they have no stable API and
//! are pinned to one exact version.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use ra_ap_ide_db::{ChangeWithProcMacros, RootDatabase};
use ra_ap_load_cargo::{LoadCargoConfig, ProcMacroServerChoice, load_workspace_at};
use ra_ap_paths::AbsPathBuf;
use ra_ap_proc_macro_api::ProcMacroClient;
use ra_ap_project_model::{CargoConfig, RustLibSource};
use ra_ap_vfs::{FileId, Vfs, VfsPath};

/// The rust-analyzer crates' version, recorded in `Analyzer.version`.
pub const RA_VERSION: &str = "0.0.356";

/// A repository loaded into rust-analyzer.
pub struct Session {
    pub(crate) db: RootDatabase,
    pub(crate) vfs: Vfs,
    root: PathBuf,
    // Dropping the client stops the proc-macro server; macros expand lazily, so it must live.
    _proc_macros: ProcMacroClient,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("root", &self.root).finish()
    }
}

impl Session {
    /// Load the cargo project at `root` (canonical path). Fails, with the reason to record, when
    /// the type-checked view would be unreliable: cargo cannot describe the workspace, the
    /// standard library's sources are missing, or the proc-macro server does not start
    /// (arch-design issue 23).
    pub fn load(root: &Path, target_dir: Option<&Path>) -> Result<Self> {
        let sysroot = Command::new("rustc")
            .arg("--print")
            .arg("sysroot")
            .current_dir(root)
            .output();
        let has_src = sysroot
            .ok()
            .filter(|o| o.status.success())
            .is_some_and(|o| {
                Path::new(String::from_utf8_lossy(&o.stdout).trim())
                    .join("lib/rustlib/src/rust/library")
                    .is_dir()
            });
        if !has_src {
            bail!("no rust-src: std is unresolved (rustup component add rust-src)");
        }
        let mut cargo = CargoConfig {
            sysroot: Some(RustLibSource::Discover),
            set_test: true,
            ..Default::default()
        };
        if let Some(dir) = target_dir {
            cargo.extra_env.insert(
                "CARGO_TARGET_DIR".into(),
                Some(dir.to_string_lossy().into_owned()),
            );
        }
        let load = LoadCargoConfig {
            load_out_dirs_from_check: true,
            with_proc_macro_server: ProcMacroServerChoice::Sysroot,
            prefill_caches: true,
            num_worker_threads: std::thread::available_parallelism().map_or(4, |n| n.get()),
            proc_macro_processes: 1,
        };
        let (db, vfs, proc_macros) = load_workspace_at(root, &cargo, &load, &|_| ())
            .context("loading the cargo workspace")?;
        let Some(proc_macros) = proc_macros else {
            bail!("the proc-macro server did not start: macros are not expanded");
        };
        Ok(Session {
            db,
            vfs,
            root: root.to_path_buf(),
            _proc_macros: proc_macros,
        })
    }

    /// rust-analyzer's id for a repository-relative file, when it loaded the file.
    pub(crate) fn file_id(&self, rel: &str) -> Option<FileId> {
        let abs = AbsPathBuf::try_from(self.root.join(rel).to_string_lossy().as_ref()).ok()?;
        self.vfs.file_id(&VfsPath::from(abs)).map(|(id, _)| id)
    }

    /// The repository-relative path of a file rust-analyzer knows, when it is in the repository.
    pub(crate) fn rel_path(&self, id: FileId) -> Option<String> {
        let path = self.vfs.file_path(id).as_path()?;
        let rel = Path::new(path.as_str()).strip_prefix(&self.root).ok()?;
        Some(rel.to_string_lossy().replace('\\', "/"))
    }

    /// Tell rust-analyzer a file's new text. Returns false when it does not know the file.
    pub fn file_changed(&mut self, rel: &str, text: String) -> bool {
        let Some(id) = self.file_id(rel) else {
            return false;
        };
        let mut change = ChangeWithProcMacros::default();
        change.change_file(id, Some(text));
        self.db.apply_change(change);
        true
    }
}
