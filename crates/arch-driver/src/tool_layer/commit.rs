//! The commit tool (ADR 0016): agents commit through arch, one commit per element.
//!
//! Only the element's files go in, whatever else is staged or changed. The message is a
//! Conventional Commit, `type(area): summary`, the body defaults to the element's intention, and
//! three trailers tie it to the plan:
//!
//! ```text
//! feat(billing): charge the card once
//!
//! Retry the charge without a second debit.
//!
//! Arch-Element: 3f2a9c1e
//! Arch-Session: s-20261003
//! Co-authored-by: Claude <noreply@anthropic.com>
//! ```
//!
//! A second call while the element's commit is HEAD amends it, so the element keeps one commit.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::{Scope, ToolLayerError};

/// The Conventional Commit types the tool accepts.
pub const TYPES: &[&str] = &[
    "feat", "fix", "refactor", "perf", "test", "docs", "style", "build", "ci", "chore", "revert",
];

/// What the agent asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRequest {
    /// `feat`, `fix`, …
    pub kind: String,
    /// One line.
    pub summary: String,
    /// The body; the element's intention when absent.
    pub body: Option<String>,
}

/// What the tool did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitOutcome {
    /// The commit.
    pub hash: String,
    /// Whether it amended the element's earlier commit.
    pub amended: bool,
    /// The files the commit changed, relative to the worktree.
    pub files: Vec<PathBuf>,
    /// The full message.
    pub message: String,
}

/// Commit the element's files in its worktree. `area` is the scope of the subject line.
pub fn commit(
    scope: &Scope,
    request: &CommitRequest,
    area: Option<&str>,
    co_author: &str,
) -> Result<CommitOutcome, ToolLayerError> {
    if !TYPES.contains(&request.kind.as_str()) {
        return Err(invalid(format!(
            "type `{}` is not one of {}",
            request.kind,
            TYPES.join(", ")
        )));
    }
    let summary = request.summary.trim();
    if summary.is_empty() || summary.contains('\n') {
        return Err(invalid("the summary is one non-empty line".into()));
    }
    let wt = &scope.worktree;
    let element = &scope.element;
    let files: Vec<String> = element
        .files
        .iter()
        .filter(|f| {
            wt.join(f).exists() || git(wt, &["ls-files", "--error-unmatch", "--"], f).is_ok()
        })
        .map(|f| f.to_string_lossy().into_owned())
        .collect();
    if files.is_empty() {
        return Err(invalid(format!(
            "element {} has no file to commit yet",
            element.label
        )));
    }
    let head_message = git_args(wt, &["log", "-1", "--format=%B"]).unwrap_or_default();
    let amend = trailer(&head_message, "Arch-Element") == Some(element.id.as_str())
        && trailer(&head_message, "Arch-Session") == Some(scope.session.as_str());

    let mut add = vec!["add", "-A", "--"];
    add.extend(files.iter().map(String::as_str));
    git_args(wt, &add)?;
    let mut diff = vec!["diff", "--cached", "--quiet", "HEAD", "--"];
    diff.extend(files.iter().map(String::as_str));
    let changed = git_args(wt, &diff).is_err();
    if !changed && !amend {
        return Err(invalid(format!(
            "nothing to commit in element {}'s files ({})",
            element.label,
            files.join(", ")
        )));
    }

    let subject = match area {
        Some(area) => format!("{}({area}): {summary}", request.kind),
        None => format!("{}: {summary}", request.kind),
    };
    let body = request
        .body
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .unwrap_or(element.intention.trim());
    let message = format!(
        "{subject}\n\n{body}\n\nArch-Element: {}\nArch-Session: {}\nCo-authored-by: {co_author}\n",
        element.id, scope.session
    );

    let mut args = vec!["commit", "--quiet", "--cleanup=whitespace", "-F", "-"];
    if amend {
        args.push("--amend");
    }
    args.push("--only");
    args.push("--");
    args.extend(files.iter().map(String::as_str));
    let mut child = Command::new("git")
        .args(&args)
        .current_dir(wt)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| ToolLayerError::Io {
            path: PathBuf::from("git"),
            source,
        })?;
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(message.as_bytes())
        .map_err(|source| ToolLayerError::Io {
            path: PathBuf::from("git"),
            source,
        })?;
    let out = child
        .wait_with_output()
        .map_err(|source| ToolLayerError::Io {
            path: PathBuf::from("git"),
            source,
        })?;
    if !out.status.success() {
        return Err(ToolLayerError::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    let hash = git_args(wt, &["rev-parse", "HEAD"])?.trim().to_string();
    let files = git_args(wt, &["show", "--name-only", "--format=", "HEAD"])?
        .lines()
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect();
    Ok(CommitOutcome {
        hash,
        amended: amend,
        files,
        message,
    })
}

/// The value of the last `key: value` trailer line of a message.
fn trailer<'a>(message: &'a str, key: &str) -> Option<&'a str> {
    message
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':').map(str::trim))
}

fn invalid(message: String) -> ToolLayerError {
    ToolLayerError::Invalid(format!("arch commit: {message}"))
}

fn git(wt: &Path, args: &[&str], path: &Path) -> Result<String, ToolLayerError> {
    let mut all: Vec<&str> = args.to_vec();
    let p = path.to_string_lossy();
    all.push(&p);
    git_args(wt, &all)
}

fn git_args(wt: &Path, args: &[&str]) -> Result<String, ToolLayerError> {
    let out = Command::new("git")
        .args(args)
        .current_dir(wt)
        .output()
        .map_err(|source| ToolLayerError::Io {
            path: PathBuf::from("git"),
            source,
        })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(ToolLayerError::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_trailer_wins() {
        let m = "feat: x\n\nbody\n\nArch-Element: aaaa\nArch-Element: bbbb\n";
        assert_eq!(trailer(m, "Arch-Element"), Some("bbbb"));
        assert_eq!(trailer(m, "Arch-Session"), None);
    }
}
