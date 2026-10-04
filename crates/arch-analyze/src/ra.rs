//! A loaded rust-analyzer: the type-checked view of the repository, kept in memory so one
//! changed file is recomputed without loading again (spec §5: one-file recompute < 500 ms).
//!
//! The `ra_ap_*` crates are rust-analyzer published as a library; they have no stable API and
//! are pinned to one exact version.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use ra_ap_ide_db::base_db::salsa::Durability;
use ra_ap_ide_db::base_db::{FileSet, SourceDatabase, SourceRoot, SourceRootId};
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

/// The Rust toolchain picked in the repository root (a committed `rust-toolchain.toml` pins
/// it), as `rustc -vV` names it. Part of every package id at resolved depth (ADR 0030).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toolchain {
    /// This machine's target triple, analysed for unless `areas.toml` pins another.
    pub host: String,
    /// `1.90.0`, `1.91.0-nightly`.
    pub release: String,
    /// The compiler's commit hash; `unknown` for a compiler built outside a git checkout.
    pub commit_hash: String,
}

impl Toolchain {
    /// Ask `rustc -vV` in `root`, where rustup honours the repository's toolchain file.
    pub fn of_repo(root: &Path) -> Result<Self> {
        let out = Command::new("rustc")
            .arg("-vV")
            .current_dir(root)
            .output()
            .context("running rustc -vV")?;
        if !out.status.success() {
            bail!("rustc -vV: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Self::parse(&String::from_utf8_lossy(&out.stdout)).context("reading rustc -vV")
    }

    /// Read `rustc -vV`'s output.
    pub fn parse(text: &str) -> Option<Self> {
        let field = |name: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(name)?.strip_prefix(": "))
                .map(|v| v.trim().to_string())
        };
        Some(Toolchain {
            host: field("host")?,
            release: field("release")?,
            commit_hash: field("commit-hash").unwrap_or_else(|| "unknown".into()),
        })
    }
}

/// A repository loaded into rust-analyzer.
pub struct Session {
    pub(crate) db: RootDatabase,
    pub(crate) vfs: Vfs,
    /// How files are grouped into source roots, as the load grouped them: a file created after
    /// the load is placed with the same rule.
    roots: SourceRootConfig,
    root: PathBuf,
    /// The triple analysed for: `areas.toml`'s `target`, else the host's.
    target: String,
    toolchain: Toolchain,
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
    /// (arch-design issue 23). `target` is the triple to analyse for; this machine's when `None`
    /// (ADR 0030).
    pub fn load(root: &Path, target_dir: Option<&Path>, target: Option<&str>) -> Result<Self> {
        let toolchain = Toolchain::of_repo(root)?;
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
            target: cargo_target(target, &toolchain.host),
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
            target: target.map_or_else(|| toolchain.host.clone(), str::to_string),
            toolchain,
            _proc_macros: proc_macros,
        })
    }

    /// The triple rust-analyzer analyses for.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The toolchain picked in the repository root at load.
    pub fn toolchain(&self) -> &Toolchain {
        &self.toolchain
    }

    /// rust-analyzer's id for a repository-relative file, when it has the file.
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
        let id = match (self.file_id(rel), &text) {
            (Some(id), _) => id,
            (None, None) => return false,
            (None, Some(text)) => match self.add_file(rel, text) {
                Some(id) => id,
                None => return false,
            },
        };
        let mut change = ChangeWithProcMacros::default();
        change.change_file(id, text);
        self.db.apply_change(change);
        true
    }

    /// Add a file created since the load to the source root of the workspace its path falls in.
    /// Only that root is set again: setting every root, libraries included, would make
    /// rust-analyzer re-check all it computed and double the next recompute.
    fn add_file(&mut self, rel: &str, text: &str) -> Option<FileId> {
        let path = self.vfs_path(rel)?;
        let set = self.roots.fsc.classify_path(&path)?;
        if !self.roots.local_filesets.contains(&(set as u64)) {
            return None;
        }
        // At load, the n-th file set of the grouping became source root n.
        let root_id = SourceRootId(u32::try_from(set).ok()?);
        let old = self.db.source_root(root_id).source_root(&self.db);
        let mut files = FileSet::default();
        for f in old.iter() {
            files.insert(f, old.path_for_file(&f)?.clone());
        }
        self.vfs
            .set_file_contents(path.clone(), Some(text.as_bytes().to_vec()));
        // The change is told to the database below; the file list's change log is not read.
        self.vfs.take_changes();
        let (id, _) = self.vfs.file_id(&path)?;
        files.insert(id, path);
        self.db
            .set_file_source_root_with_durability(id, root_id, Durability::LOW);
        self.db.set_source_root_with_durability(
            root_id,
            SourceRoot::new_local(files).into(),
            Durability::LOW,
        );
        Some(id)
    }
}

/// The `--target` rust-analyzer gives cargo: none when the pin is this machine's own triple.
/// The platform analysed for is the same, and cargo keeps a `--target` build apart from the
/// plain one (`target/<triple>/`), so passing it would rebuild what the developer already built
/// and make the first index cold (ADR 0026).
fn cargo_target(target: Option<&str>, host: &str) -> Option<String> {
    target.filter(|t| *t != host).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_to_this_machine_s_triple_builds_where_the_developer_did() {
        let linux = "x86_64-unknown-linux-gnu";
        assert_eq!(cargo_target(Some(linux), linux), None);
        assert_eq!(
            cargo_target(Some(linux), "aarch64-apple-darwin").as_deref(),
            Some(linux)
        );
        assert_eq!(cargo_target(None, linux), None);
    }

    #[test]
    fn rustc_vv_names_the_host_release_and_commit() {
        let text = "rustc 1.99.0 (b940084d7 2026-09-28)\nbinary: rustc\ncommit-hash: b940084d7eb6a299eb4bfeb8e34901bc051e7ac4\ncommit-date: 2026-09-28\nhost: aarch64-apple-darwin\nrelease: 1.99.0\nLLVM version: 23.1.1\n";
        assert_eq!(
            Toolchain::parse(text),
            Some(Toolchain {
                host: "aarch64-apple-darwin".into(),
                release: "1.99.0".into(),
                commit_hash: "b940084d7eb6a299eb4bfeb8e34901bc051e7ac4".into(),
            })
        );
        assert_eq!(Toolchain::parse("rustc 1.99.0\n"), None);
    }
}
