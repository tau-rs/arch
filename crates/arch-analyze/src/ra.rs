//! A loaded rust-analyzer: the type-checked view of the repository, kept in memory so one
//! changed file is recomputed without loading again (spec §5: one-file recompute < 500 ms).
//!
//! The `ra_ap_*` crates are rust-analyzer published as a library; they have no stable API and
//! are pinned to one exact version.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use ra_ap_ide_db::{ChangeWithProcMacros, RootDatabase};
use ra_ap_load_cargo::{
    LoadCargoConfig, ProcMacroServerChoice, ProjectFolders, SourceRootConfig, load_workspace,
};
use ra_ap_paths::AbsPathBuf;
use ra_ap_proc_macro_api::ProcMacroClient;
use ra_ap_project_model::{CargoConfig, ProjectManifest, ProjectWorkspace, RustLibSource};
use ra_ap_vfs::{FileId, Vfs, VfsPath};

/// The rust-analyzer crates' version, recorded in `Analyzer.version`.
pub const RA_VERSION: &str = "0.0.356";

/// A repository loaded into rust-analyzer.
pub struct Session {
    pub(crate) db: RootDatabase,
    pub(crate) vfs: Vfs,
    /// How files are grouped into source roots, as the load grouped them: a file created after
    /// the load is placed with the same rule.
    roots: SourceRootConfig,
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
        // `load_workspace_at`, unrolled to keep the source-root grouping it computes and drops.
        let (workspace, roots) = (|| {
            let root = AbsPathBuf::try_from(root.to_string_lossy().as_ref())
                .map_err(|p| anyhow::anyhow!("not an absolute UTF-8 path: {p}"))?;
            let manifest = ProjectManifest::discover_single(&root)?;
            let mut workspace = ProjectWorkspace::load(manifest, &cargo, &|_| ())?;
            let scripts = workspace.run_build_scripts(&cargo, &|_| ())?;
            workspace.set_build_scripts(scripts);
            let roots =
                ProjectFolders::new(std::slice::from_ref(&workspace), &[], None).source_root_config;
            anyhow::Ok((workspace, roots))
        })()
        .context("loading the cargo workspace")?;
        let (db, vfs, proc_macros) = load_workspace(workspace, &cargo.extra_env, &load)
            .context("loading the cargo workspace")?;
        let Some(proc_macros) = proc_macros else {
            bail!("the proc-macro server did not start: macros are not expanded");
        };
        Ok(Session {
            db,
            vfs,
            roots,
            root: root.to_path_buf(),
            _proc_macros: proc_macros,
        })
    }

    /// rust-analyzer's id for a repository-relative file, when it loaded the file.
    pub(crate) fn file_id(&self, rel: &str) -> Option<FileId> {
        self.vfs.file_id(&self.vfs_path(rel)?).map(|(id, _)| id)
    }

    fn vfs_path(&self, rel: &str) -> Option<VfsPath> {
        let abs = AbsPathBuf::try_from(self.root.join(rel).to_string_lossy().as_ref()).ok()?;
        Some(VfsPath::from(abs))
    }

    /// The repository-relative path of a file rust-analyzer knows, when it is in the repository.
    pub(crate) fn rel_path(&self, id: FileId) -> Option<String> {
        let path = self.vfs.file_path(id).as_path()?;
        let rel = Path::new(path.as_str()).strip_prefix(&self.root).ok()?;
        Some(rel.to_string_lossy().replace('\\', "/"))
    }

    /// Tell rust-analyzer a file's new text, or `None` when the file is gone. A file created
    /// since the load joins the source root its path falls in, as if it had been there at load.
    /// Returns false when rust-analyzer does not know the file and cannot take it: it is outside
    /// every source root of the workspace.
    pub fn file_changed(&mut self, rel: &str, text: Option<String>) -> bool {
        let mut change = ChangeWithProcMacros::default();
        match (self.file_id(rel), text) {
            (Some(id), text) => change.change_file(id, text),
            (None, None) => return false,
            (None, Some(text)) => {
                let Some(path) = self.vfs_path(rel) else {
                    return false;
                };
                let local = self
                    .roots
                    .fsc
                    .classify_path(&path)
                    .is_some_and(|i| self.roots.local_filesets.contains(&(i as u64)));
                if !local {
                    return false;
                }
                self.vfs
                    .set_file_contents(path.clone(), Some(text.clone().into_bytes()));
                let Some((id, _)) = self.vfs.file_id(&path) else {
                    return false;
                };
                // The change is applied below; the file system's change log is not read.
                self.vfs.take_changes();
                change.change_file(id, Some(text));
                change.set_roots(self.roots.partition(&self.vfs));
            }
        }
        self.db.apply_change(change);
        true
    }
}
