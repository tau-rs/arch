//! Git reader: pointers, merge base, the tree's files, and commits as facts with their trailers
//! (ADR 0016). Shells out to `git`; a directory that is not a repository reads as "no git".

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use arch_facts::{Commit, Trailer};

/// The trailer that names the plan element a commit realizes (ADR 0021).
pub const ELEMENT_TRAILER: &str = "Arch-Element";

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .context("running git")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether `root` is inside a git work tree.
pub fn is_repo(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|o| o.trim() == "true")
}

/// The full hash a revision points at (`HEAD`, a branch, a tag).
pub fn rev_parse(root: &Path, rev: &str) -> Result<String> {
    Ok(git(
        root,
        &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
    )?
    .trim()
    .to_string())
}

/// The current branch, when HEAD is on one.
pub fn current_branch(root: &Path) -> Option<String> {
    let b = git(root, &["symbolic-ref", "--short", "-q", "HEAD"]).ok()?;
    let b = b.trim();
    (!b.is_empty()).then(|| b.to_string())
}

/// The merge base of two revisions.
pub fn merge_base(root: &Path, a: &str, b: &str) -> Result<String> {
    Ok(git(root, &["merge-base", a, b])?.trim().to_string())
}

/// The branch the repository integrates into: `origin/HEAD`, else `main`, else `master`.
pub fn default_branch(root: &Path) -> Option<String> {
    if let Ok(r) = git(
        root,
        &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"],
    ) {
        let r = r.trim();
        if !r.is_empty() {
            return Some(r.to_string());
        }
    }
    ["main", "master"]
        .into_iter()
        .find(|b| rev_parse(root, b).is_ok())
        .map(str::to_string)
}

/// Whether anything under `root` differs from HEAD (tracked changes or untracked files).
pub fn is_dirty(root: &Path) -> Result<bool> {
    Ok(!git(root, &["status", "--porcelain", "--", "."])?
        .trim()
        .is_empty())
}

/// Files under `root`, tracked or untracked and not ignored, relative to `root`, sorted.
pub fn files(root: &Path) -> Result<Vec<PathBuf>> {
    let out = git(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            ".",
        ],
    )?;
    // `git -C root ls-files` prints paths relative to root.
    let mut files: Vec<PathBuf> = out
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .filter(|p| root.join(p).is_file())
        .collect();
    files.sort();
    files.dedup();
    Ok(files)
}

/// Commits in `range` (`base..head`, or a single revision for its whole history), oldest first,
/// limited to those touching `root`, with file paths relative to `root`.
pub fn commits(root: &Path, range: &str) -> Result<Vec<Commit>> {
    let format = "--format=%x1e%H%x1f%an <%ae>%x1f%s%x1f%(trailers:only,unfold)%x1f";
    let out = git(
        root,
        &[
            "log",
            "--reverse",
            "--name-only",
            "--relative",
            format,
            range,
            "--",
            ".",
        ],
    )?;
    let mut commits = Vec::new();
    for record in out.split('\x1e').filter(|r| !r.trim().is_empty()) {
        let mut f = record.split('\x1f');
        let (Some(hash), Some(author), Some(summary), Some(trailers), Some(files)) =
            (f.next(), f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        let trailers = parse_trailers(trailers);
        let element = trailers
            .iter()
            .find(|t| t.key == ELEMENT_TRAILER)
            .map(|t| t.value.clone());
        commits.push(Commit {
            hash: hash.trim().to_string(),
            author: author.to_string(),
            summary: summary.to_string(),
            trailers,
            files: files
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
            element,
        });
    }
    Ok(commits)
}

fn parse_trailers(block: &str) -> Vec<Trailer> {
    block
        .lines()
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| Trailer {
            key: k.trim().to_string(),
            value: v.trim().to_string(),
        })
        .filter(|t| !t.key.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=Ada",
                "-c",
                "user.email=ada@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            ok.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&ok.stderr)
        );
    }

    #[test]
    fn commits_carry_author_trailers_files_and_the_plan_element() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run(dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.rs"), "fn a() {}\n").unwrap();
        run(dir, &["add", "."]);
        run(dir, &["commit", "-q", "-m", "feat: seed"]);
        let base = rev_parse(dir, "HEAD").unwrap();
        run(dir, &["checkout", "-q", "-b", "work"]);
        std::fs::create_dir(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/b.rs"), "fn b() {}\n").unwrap();
        run(dir, &["add", "."]);
        run(
            dir,
            &[
                "commit",
                "-q",
                "-m",
                "feat(pay): capture\n\nBody line.\n\nArch-Element: 1a2b3c4d\nCo-Authored-By: Bo <bo@example.com>",
            ],
        );
        assert!(is_repo(dir));
        assert_eq!(current_branch(dir).as_deref(), Some("work"));
        assert_eq!(default_branch(dir).as_deref(), Some("main"));
        assert_eq!(merge_base(dir, "main", "HEAD").unwrap(), base);
        assert!(!is_dirty(dir).unwrap());

        let got = commits(dir, &format!("{base}..HEAD")).unwrap();
        assert_eq!(got.len(), 1);
        let c = &got[0];
        assert_eq!(c.author, "Ada <ada@example.com>");
        assert_eq!(c.summary, "feat(pay): capture");
        assert_eq!(c.files, ["src/b.rs"]);
        assert_eq!(c.element.as_deref(), Some("1a2b3c4d"));
        assert_eq!(c.trailers.len(), 2);
        assert_eq!(c.trailers[1].key, "Co-Authored-By");

        let all = commits(dir, "HEAD").unwrap();
        assert_eq!(
            all.iter().map(|c| c.summary.as_str()).collect::<Vec<_>>(),
            ["feat: seed", "feat(pay): capture"]
        );

        std::fs::write(dir.join("untracked.rs"), "").unwrap();
        assert!(is_dirty(dir).unwrap());
        assert_eq!(
            files(dir).unwrap(),
            [
                PathBuf::from("a.rs"),
                "src/b.rs".into(),
                "untracked.rs".into()
            ]
        );
    }

    #[test]
    fn a_plain_directory_is_not_a_repository() {
        let tmp = tempfile::tempdir().unwrap();
        // A temp dir may sit inside a repo on some machines; only assert when it does not.
        if !is_repo(tmp.path()) {
            assert!(files(tmp.path()).is_err());
        }
    }
}
