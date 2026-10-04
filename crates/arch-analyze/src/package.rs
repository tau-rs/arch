//! Package ids: what names a file's facts besides its path and content (ADR 0002).
//!
//! A package (one `Cargo.toml` and its directory) is named the way git names it: by the id of
//! its directory's git tree, the one `git write-tree` gives for the working files (or
//! `git rev-parse <commit>:<dir>` once committed). The ids are computed here, in git's object
//! format, from the bytes the analyzer reads anyway: no `git` process, and nothing written into
//! the repository's object store. A package id then combines that tree id with the tree ids of
//! the unit packages it depends on (all of them, transitively, so that cycles through
//! dev-dependencies are harmless), the `Cargo.lock` blob id, what the analyzer was asked for
//! (its version and depth, the unit, what it could not type-check, the target and toolchain),
//! and the content of the build-script output of the package and of those it depends on
//! (ADR 0030).
//!
//! Git-compatible within limits stated here: submodules (gitlinks) and symlinks to directories
//! are not among the files the analyzer reads, so a tree that holds them gets another id than
//! git's. The key stays a function of what the facts are computed from.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::cargo::UnitPlan;

/// The hash a repository names its objects with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectFormat {
    /// SHA-1, git's default.
    #[default]
    Sha1,
    /// SHA-256 (`extensions.objectFormat = sha256`).
    Sha256,
}

impl ObjectFormat {
    /// The format of the repository at `root`; SHA-1 outside a repository.
    pub fn of_repo(root: &Path) -> Self {
        let out = std::process::Command::new("git")
            .args(["rev-parse", "--show-object-format"])
            .current_dir(root)
            .output();
        match out {
            Ok(o) if o.status.success() && o.stdout.starts_with(b"sha256") => Self::Sha256,
            _ => Self::Sha1,
        }
    }

    fn object(self, kind: &str, body: &[u8]) -> Vec<u8> {
        let header = format!("{kind} {}\0", body.len());
        match self {
            Self::Sha1 => {
                let mut h = Sha1::new();
                h.update(header.as_bytes());
                h.update(body);
                h.finalize().to_vec()
            }
            Self::Sha256 => {
                let mut h = Sha256::new();
                h.update(header.as_bytes());
                h.update(body);
                h.finalize().to_vec()
            }
        }
    }
}

/// One file as git stores it: its mode and blob id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    /// `100644`, `100755` or `120000`.
    pub mode: &'static str,
    /// Raw object id.
    pub id: Vec<u8>,
}

impl Blob {
    /// The blob of a file whose content is `bytes`, with its mode read from the file system:
    /// executable when its owner may run it; a symlink stores its target.
    pub fn of_file(format: ObjectFormat, path: &Path, bytes: &[u8]) -> std::io::Result<Self> {
        let meta = std::fs::symlink_metadata(path)?;
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(path)?;
            let target = target.to_string_lossy().replace('\\', "/");
            return Ok(Blob {
                mode: "120000",
                id: format.object("blob", target.as_bytes()),
            });
        }
        Ok(Blob {
            mode: if executable(&meta) {
                "100755"
            } else {
                "100644"
            },
            id: format.object("blob", bytes),
        })
    }

    /// The blob id, hex.
    pub fn hex(&self) -> String {
        hex::encode(&self.id)
    }
}

#[cfg(unix)]
fn executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o100 != 0
}

#[cfg(not(unix))]
fn executable(_: &std::fs::Metadata) -> bool {
    false
}

#[derive(Default)]
struct Dir {
    files: BTreeMap<String, Blob>,
    dirs: BTreeMap<String, Dir>,
}

/// The git tree id of every directory holding one of `files`, by repository-relative path (the
/// root is the empty path).
pub fn tree_ids(format: ObjectFormat, files: &[(PathBuf, Blob)]) -> BTreeMap<PathBuf, String> {
    let mut root = Dir::default();
    for (path, blob) in files {
        let parts: Vec<String> = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let Some((name, dirs)) = parts.split_last() else {
            continue;
        };
        let mut at = &mut root;
        for d in dirs {
            at = at.dirs.entry(d.clone()).or_default();
        }
        at.files.insert(name.clone(), blob.clone());
    }
    let mut out = BTreeMap::new();
    tree(format, &root, PathBuf::new(), &mut out);
    out
}

fn tree(
    format: ObjectFormat,
    dir: &Dir,
    path: PathBuf,
    out: &mut BTreeMap<PathBuf, String>,
) -> Vec<u8> {
    // Git sorts a tree's entries by name, a directory's name compared as if it ended in `/`.
    let mut entries: Vec<(Vec<u8>, &str, &str, Vec<u8>)> = Vec::new();
    for (name, blob) in &dir.files {
        entries.push((name.as_bytes().to_vec(), name, blob.mode, blob.id.clone()));
    }
    for (name, sub) in &dir.dirs {
        let id = tree(format, sub, path.join(name), out);
        let mut key = name.as_bytes().to_vec();
        key.push(b'/');
        entries.push((key, name, "40000", id));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut body = Vec::new();
    for (_, name, mode, id) in entries {
        body.extend_from_slice(mode.as_bytes());
        body.push(b' ');
        body.extend_from_slice(name.as_bytes());
        body.push(0);
        body.extend_from_slice(&id);
    }
    let id = format.object("tree", &body);
    out.insert(path, hex::encode(&id));
    id
}

/// Package ids of a unit, by index into `plan.packages`, and the unit's own id for files no
/// package holds.
#[derive(Debug, Clone)]
pub struct PackageIds {
    /// One per package of the plan.
    pub packages: Vec<String>,
    /// For files outside every package: the root tree, the lock and the analyzer.
    pub unit: String,
}

/// Name every package of `plan` (ADR 0002). `trees` are the tree ids from [`tree_ids`], `lock`
/// the `Cargo.lock` blob id, `analyzer` what the analyzer was asked for and could do, and
/// `out_dirs` the hash of each package's build-script output by package directory, for the
/// packages that have one (ADR 0030).
pub fn package_ids(
    plan: &UnitPlan,
    trees: &BTreeMap<PathBuf, String>,
    lock: Option<&str>,
    analyzer: &str,
    out_dirs: &BTreeMap<PathBuf, String>,
) -> PackageIds {
    let empty = String::from("-");
    let tree_of = |p: usize| trees.get(&package_dir(plan, p)).unwrap_or(&empty);
    let by_name: BTreeMap<&str, usize> = plan
        .packages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.name.as_str(), i))
        .collect();
    let tail = format!("lock {}\n{analyzer}", lock.unwrap_or("-"));
    let packages = (0..plan.packages.len())
        .map(|p| {
            // Everything it depends on in the unit, transitively.
            let mut seen: BTreeSet<usize> = BTreeSet::new();
            let mut queue = vec![p];
            while let Some(q) = queue.pop() {
                for d in &plan.packages[q].deps {
                    if let Some(&r) = by_name.get(d.package.as_str())
                        && r != p
                        && seen.insert(r)
                    {
                        queue.push(r);
                    }
                }
            }
            let mut text = format!("arch package 1\ntree {}\n", tree_of(p));
            let deps: BTreeMap<&str, &String> = seen
                .iter()
                .map(|r| (plan.packages[*r].name.as_str(), tree_of(*r)))
                .collect();
            for (name, t) in deps {
                text.push_str(&format!("dep {name} {t}\n"));
            }
            // What build scripts generated, read like source: the package's own and its deps'.
            let outs: BTreeMap<&str, &String> = std::iter::once(p)
                .chain(seen.iter().copied())
                .filter_map(|r| {
                    let out = out_dirs.get(&package_dir(plan, r))?;
                    Some((plan.packages[r].name.as_str(), out))
                })
                .collect();
            for (name, out) in outs {
                text.push_str(&format!("out_dir {name} {out}\n"));
            }
            text.push_str(&tail);
            hex::encode(Sha256::digest(text.as_bytes()))
        })
        .collect();
    let root = trees.get(Path::new("")).unwrap_or(&empty);
    let unit = hex::encode(Sha256::digest(
        format!("arch unit 1\ntree {root}\n{tail}").as_bytes(),
    ));
    PackageIds { packages, unit }
}

/// A package's directory, relative to the repository root.
pub fn package_dir(plan: &UnitPlan, package: usize) -> PathBuf {
    plan.packages[package]
        .manifest
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_and_tree_ids_are_gits() {
        // `printf 'hello\n' | git hash-object --stdin`, and the tree of that one file as
        // `git write-tree` names it.
        let blob = Blob {
            mode: "100644",
            id: ObjectFormat::Sha1.object("blob", b"hello\n"),
        };
        assert_eq!(blob.hex(), "ce013625030ba8dba906f756967f9e9ca394464a");
        let trees = tree_ids(ObjectFormat::Sha1, &[(PathBuf::from("a/hello"), blob)]);
        assert_eq!(
            trees[Path::new("a")],
            "b4d01e9b0c4a9356736dfddf8830ba9a54f5271c"
        );
        // The empty tree.
        assert_eq!(
            hex::encode(ObjectFormat::Sha1.object("tree", b"")),
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
        );
    }

    #[test]
    fn a_directory_sorts_as_if_its_name_ended_in_a_slash() {
        let b = |s: &str| Blob {
            mode: "100644",
            id: ObjectFormat::Sha1.object("blob", s.as_bytes()),
        };
        // Git orders `a.rs` < `a/` < `a0`; by plain names `a` would come first. The id is
        // `git write-tree` of these three files.
        let files = [
            (PathBuf::from("a.rs"), b("1")),
            (PathBuf::from("a/x"), b("2")),
            (PathBuf::from("a0"), b("3")),
        ];
        assert_eq!(
            tree_ids(ObjectFormat::Sha1, &files)[Path::new("")],
            "3b44fbf44b8c25378e39bbb06d20469e9ecfb519"
        );
    }
}
