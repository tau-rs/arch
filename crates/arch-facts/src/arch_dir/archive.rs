//! The archive (ADR 0003): at merge, a session's folder moves from its branch to a git note
//! on main under `refs/notes/arch`. An archived session is restorable from the note.
//!
//! The note is one TOML document holding the folder's files verbatim. The commit the note is
//! attached to is the caller's choice (the merge commit on main); arch-design#19 asks the product
//! chat to fix both.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::session::{SessionId, Timestamp, now};

use super::SessionDir;

/// The notes ref (ADR 0003).
pub const NOTES_REF: &str = "refs/notes/arch";

/// One file of an archived session folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveFile {
    /// Path relative to `sessions/<id>/`.
    pub path: PathBuf,
    /// Content.
    pub content: String,
}

/// An archived session: the folder's files and when it was archived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Archive {
    /// The session.
    pub session: SessionId,
    /// When.
    pub archived_at: Timestamp,
    /// Files.
    #[serde(rename = "file", default)]
    pub files: Vec<ArchiveFile>,
}

impl Archive {
    /// Capture a session folder.
    pub fn of(dir: &SessionDir) -> Result<Self> {
        let files = dir
            .files()?
            .into_iter()
            .map(|(path, content)| ArchiveFile { path, content })
            .collect();
        Ok(Archive {
            session: dir.id().clone(),
            archived_at: now(),
            files,
        })
    }

    /// Serialize as the note's text.
    pub fn to_toml(&self) -> Result<String> {
        super::to_toml(self)
    }

    /// Parse a note's text.
    pub fn from_toml(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|e| Error::format("refs/notes/arch", e.message()))
    }

    /// Write the note on `commit` in `repo`, replacing any earlier note there.
    pub fn write_note(&self, repo: &Path, commit: &str) -> Result<()> {
        let text = self.to_toml()?;
        git(
            repo,
            &["notes", "--ref", NOTES_REF, "add", "-f", "-F", "-", commit],
            Some(&text),
        )?;
        Ok(())
    }

    /// Read the note on `commit`, if any.
    pub fn read_note(repo: &Path, commit: &str) -> Result<Option<Self>> {
        match git(repo, &["notes", "--ref", NOTES_REF, "show", commit], None) {
            Ok(text) => Ok(Some(Self::from_toml(&text)?)),
            Err(Error::Git { stderr, .. }) if stderr.contains("no note found") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Write the files back under a session folder (restore).
    pub fn restore_to(&self, dir: &SessionDir) -> Result<()> {
        for f in &self.files {
            let p = dir.root().join(&f.path);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
            }
            std::fs::write(&p, &f.content).map_err(|e| Error::io(&p, e))?;
        }
        Ok(())
    }

    /// Archive a live session (ADR 0003): capture the folder, write the note on `commit`,
    /// remove the folder from the tree. The caller commits the removal.
    pub fn move_to_notes(dir: &SessionDir, repo: &Path, commit: &str) -> Result<Self> {
        let archive = Self::of(dir)?;
        archive.write_note(repo, commit)?;
        dir.remove()?;
        Ok(archive)
    }
}

fn git(repo: &Path, args: &[&str], stdin: Option<&str>) -> Result<String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = cmd.spawn().map_err(|e| Error::Git {
        args: args.join(" "),
        stderr: e.to_string(),
    })?;
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .expect("piped")
            .write_all(text.as_bytes())
            .map_err(|e| Error::Git {
                args: args.join(" "),
                stderr: e.to_string(),
            })?;
    }
    let out = child.wait_with_output().map_err(|e| Error::Git {
        args: args.join(" "),
        stderr: e.to_string(),
    })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(Error::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }
}
