//! `arch init` (ADR 0006): `areas.toml` with computed sides and order, `rules` from the template,
//! the gitignore line for `.arch/cache/`; one commit; no questions.

use std::path::{Path, PathBuf};
use std::process::Command;

use arch_analyze::{Commits, Options, analyze};
use arch_facts::arch_dir::GITIGNORE_LINE;
use arch_facts::{ArchDir, Areas, Rules};
use arch_views::propose_areas;

use crate::Error;

/// Options for [`init`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitOptions {
    /// Commit the written files (the default, ADR 0006). Off for a throwaway clone, or to look
    /// at the files first.
    pub commit: bool,
}

impl Default for InitOptions {
    fn default() -> Self {
        InitOptions { commit: true }
    }
}

/// What [`init`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitOutcome {
    /// The areas written to `areas.toml`.
    pub areas: Areas,
    /// The files written or changed, relative to the repository.
    pub written: Vec<PathBuf>,
    /// The commit made, when one was asked for.
    pub commit: Option<String>,
}

/// Write `.arch/` for the repository at `repo`.
///
/// Refuses a repository that already has `areas.toml` or `rules`: after `init` the files are the
/// record and running it again is not defined (ADR 0029).
pub fn init(repo: &Path, options: &InitOptions) -> Result<InitOutcome, Error> {
    let arch = ArchDir::of_repo(repo);
    if arch.areas_path().exists() || arch.rules_path().exists() {
        return Err(Error::AlreadyInitialized(repo.to_path_buf()));
    }
    if options.commit {
        git(repo, &["rev-parse", "--is-inside-work-tree"]).map_err(|_| {
            Error::Git(format!(
                "{} is not a git repository; pass --no-commit to write the files only",
                repo.display()
            ))
        })?;
    }
    let facts = analyze(
        repo,
        &Options {
            commits: Commits::None,
            ..Options::default()
        },
    )?;
    let areas = propose_areas(&facts);

    std::fs::create_dir_all(arch.root()).map_err(|e| io(arch.root(), e))?;
    arch.write_areas(&areas)?;
    arch.write_rules(&Rules::v1_template())?;
    let mut written = vec![
        PathBuf::from(".arch/areas.toml"),
        PathBuf::from(".arch/rules"),
    ];
    if add_gitignore_line(repo)? {
        written.push(PathBuf::from(".gitignore"));
    }

    let commit = if options.commit {
        let paths: Vec<&str> = written.iter().filter_map(|p| p.to_str()).collect();
        git(repo, &[&["add", "--"], paths.as_slice()].concat())?;
        git(
            repo,
            &[
                &[
                    "commit",
                    "--quiet",
                    "-m",
                    "chore(arch): add .arch (areas, rules)",
                    "--",
                ],
                paths.as_slice(),
            ]
            .concat(),
        )?;
        Some(git(repo, &["rev-parse", "HEAD"])?)
    } else {
        None
    };
    Ok(InitOutcome {
        areas,
        written,
        commit,
    })
}

/// Add the `.arch/cache/` line to `.gitignore` when it is not there; whether the file changed.
fn add_gitignore_line(repo: &Path) -> Result<bool, Error> {
    let path = repo.join(".gitignore");
    let mut text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io(&path, e)),
    };
    if text.lines().any(|l| l.trim() == GITIGNORE_LINE) {
        return Ok(false);
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(GITIGNORE_LINE);
    text.push('\n');
    std::fs::write(&path, text).map_err(|e| io(&path, e))?;
    Ok(true)
}

fn git(repo: &Path, args: &[&str]) -> Result<String, Error> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| Error::Git(format!("could not run git: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(Error::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

fn io(path: &Path, source: std::io::Error) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        source,
    }
}
