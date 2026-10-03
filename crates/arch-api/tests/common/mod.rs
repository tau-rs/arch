//! A scratch copy of the sample service from the pinned arch-fixtures checkout.
#![allow(dead_code)] // each test binary uses its own subset

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Copy `fixtures/arch-fixtures/repos/smallsvc` into a temp dir (without `target/`).
pub fn smallsvc_copy() -> tempfile::TempDir {
    let src = repo_root().join("fixtures/arch-fixtures/repos/smallsvc");
    assert!(
        src.join("Cargo.toml").is_file(),
        "{} is missing: run scripts/fetch-fixtures.sh",
        src.display()
    );
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&src, dir.path());
    dir
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        let dest = to.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

/// Run git in `dir` and return stdout; panics on failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Make `dir` a git repository with an identity and one commit holding everything in it.
pub fn git_init(dir: &Path) {
    git(dir, &["init", "--quiet", "-b", "main"]);
    git(dir, &["config", "user.name", "arch test"]);
    git(dir, &["config", "user.email", "arch-test@example.invalid"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "--quiet", "-m", "initial"]);
}
