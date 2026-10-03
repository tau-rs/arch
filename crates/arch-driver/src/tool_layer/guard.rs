//! The decisions of the tool layer, as pure functions over (tool call, scope, state, the file on
//! disk). The pipeline of `arch hook pre` (spec §8 keeps its seams for V2 plugins):
//!
//! 1. **stale-write guard**: a file that exists must have this element's last-read hash; a file
//!    never read is denied ("read it first"), a changed one too ("re-read it"); a new file passes;
//! 2. **element-scope veto**: the path, relative to the worktree, is one of `element.files`;
//! 3. a write both let through is **expected** by content (ADR 0012), so the watcher knows it.
//!
//! `Bash` is checked against a short list instead (FINDINGS F-5): cargo check · build · test ·
//! clippy · `fmt --check`, read-only git, and `head` · `tail` · `grep` · `wc` as filters. `git
//! commit` and `git push` are denied by name (ADR 0016, 0017).

use std::path::{Component, Path, PathBuf};

use arch_facts::{ContentHash, ToolLayerState};
use serde_json::Value;

use super::Scope;

/// The built-in tools that write a file, and the input field naming it.
pub const WRITE_TOOLS: &[(&str, &str)] = &[
    ("Edit", "file_path"),
    ("Write", "file_path"),
    ("MultiEdit", "file_path"),
    ("NotebookEdit", "notebook_path"),
];

/// A tool call as a hook sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// `Edit`, `Write`, `Read`, `Bash`, …
    pub tool: String,
    /// The tool's input.
    pub input: Value,
}

/// What the pre hook decides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Let the call through; a write's expected content, when it can be computed.
    Allow {
        /// (worktree-relative path, content hash once written).
        expect: Option<(PathBuf, ContentHash)>,
    },
    /// Block the call.
    Deny(Denial),
}

/// A blocked call: what the model reads and what the Denial record keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denial {
    /// The worktree-relative path refused, for a write; `None` for a command.
    pub path: Option<PathBuf>,
    /// arch's reason, shown to the model.
    pub reason: String,
    /// The agent's stated reason (a Bash call's `description`).
    pub agent_reason: Option<String>,
}

/// What the guard needs from the file system: where a path lands, and a file's bytes.
pub trait Disk {
    /// The absolute path `raw` resolves to.
    fn locate(&self, raw: &Path) -> PathBuf;
    /// The bytes at a worktree-relative path, `None` when absent.
    fn read(&self, rel: &Path) -> Option<Vec<u8>>;
}

impl Disk for Scope {
    fn locate(&self, raw: &Path) -> PathBuf {
        Scope::locate(self, raw)
    }
    fn read(&self, rel: &Path) -> Option<Vec<u8>> {
        self.on_disk(rel)
    }
}

/// `arch hook pre`: decide on one tool call.
pub fn pre(call: &ToolCall, scope: &Scope, state: &ToolLayerState, disk: &dyn Disk) -> Verdict {
    if call.tool == "Bash" {
        let command = call.input["command"].as_str().unwrap_or_default();
        return match bash(command) {
            Ok(()) => Verdict::Allow { expect: None },
            Err(reason) => Verdict::Deny(Denial {
                path: None,
                reason,
                agent_reason: call.input["description"].as_str().map(str::to_string),
            }),
        };
    }
    let Some(field) = write_field(&call.tool) else {
        return Verdict::Allow { expect: None };
    };
    let rel = match target(call, field, scope, disk) {
        Ok(rel) => rel,
        Err(reason) => {
            return Verdict::Deny(Denial {
                path: None,
                reason,
                agent_reason: None,
            });
        }
    };
    let deny = |reason: String| {
        Verdict::Deny(Denial {
            path: Some(rel.clone()),
            reason,
            agent_reason: None,
        })
    };
    let label = &scope.element.label;
    let current = disk.read(&rel);
    // 1. The stale-write guard.
    if let Some(bytes) = &current {
        match state.last_read(&rel) {
            None => {
                return deny(format!(
                    "arch: {} exists and element {label} has not read it; read it first \
                     (Read or mcp__arch__read), then retry",
                    rel.display()
                ));
            }
            Some(seen) if *seen != ContentHash::of_bytes(bytes) => {
                return deny(format!(
                    "arch: {} changed since element {label} last read it; re-read it, then retry",
                    rel.display()
                ));
            }
            Some(_) => {}
        }
    }
    // 2. The element-scope veto.
    if !scope.element.files.iter().any(|f| normalize(f) == rel) {
        let may: Vec<String> = scope
            .element
            .files
            .iter()
            .map(|f| f.display().to_string())
            .collect();
        return deny(format!(
            "arch: {} is outside element {label} ({}); it may write: {}. If the change is \
             needed, ask (mcp__arch__ask) instead of working around it",
            rel.display(),
            scope.element.id,
            if may.is_empty() {
                "nothing".to_string()
            } else {
                may.join(", ")
            }
        ));
    }
    // 3. Expect the content, for the watcher.
    let expect =
        written(call, current.as_deref()).map(|bytes| (rel, ContentHash::of_bytes(&bytes)));
    Verdict::Allow { expect }
}

/// `arch hook post`: record what the call left on disk. `failed` is true for
/// `PostToolUseFailure`.
pub fn post(
    call: &ToolCall,
    failed: bool,
    scope: &Scope,
    state: &mut ToolLayerState,
    disk: &dyn Disk,
) {
    let field = if call.tool == "Read" {
        "file_path"
    } else if let Some(field) = write_field(&call.tool) {
        field
    } else {
        return;
    };
    let Ok(rel) = target(call, field, scope, disk) else {
        return;
    };
    let on_disk = disk.read(&rel).map(|b| ContentHash::of_bytes(&b));
    match (call.tool.as_str(), failed, on_disk) {
        ("Read", false, Some(hash)) => state.record_read(&rel, hash),
        ("Read", _, _) => {}
        (_, false, Some(hash)) => state.confirm(&rel, hash),
        (_, _, hash) => state.drop_failed(&rel, hash.as_ref()),
    }
}

fn write_field(tool: &str) -> Option<&'static str> {
    WRITE_TOOLS
        .iter()
        .find(|(name, _)| *name == tool)
        .map(|(_, field)| *field)
}

/// The worktree-relative path a call targets, or the reason it has none.
fn target(call: &ToolCall, field: &str, scope: &Scope, disk: &dyn Disk) -> Result<PathBuf, String> {
    let raw = call.input[field]
        .as_str()
        .ok_or_else(|| format!("arch: {} without a {field}", call.tool))?;
    let at = disk.locate(Path::new(raw));
    at.strip_prefix(&scope.worktree)
        .map(Path::to_path_buf)
        .map_err(|_| {
            format!(
                "arch: {raw} is outside the worktree {}",
                scope.worktree.display()
            )
        })
}

/// The file's content after the call, when it can be computed: `Write` carries it, `Edit` and
/// `MultiEdit` are old → new on the current content. `None` when it cannot (the tool will fail,
/// or it is a notebook); the post hook then confirms the hash on disk.
fn written(call: &ToolCall, current: Option<&[u8]>) -> Option<Vec<u8>> {
    let input = &call.input;
    match call.tool.as_str() {
        "Write" => Some(input["content"].as_str()?.as_bytes().to_vec()),
        "Edit" => {
            let text = current
                .map(|b| String::from_utf8(b.to_vec()))
                .transpose()
                .ok()?;
            edit(text, input).map(String::into_bytes)
        }
        "MultiEdit" => {
            let mut text = current
                .map(|b| String::from_utf8(b.to_vec()))
                .transpose()
                .ok()?;
            for e in input["edits"].as_array()? {
                text = Some(edit(text, e)?);
            }
            text.map(String::into_bytes)
        }
        _ => None,
    }
}

fn edit(current: Option<String>, input: &Value) -> Option<String> {
    let old = input["old_string"].as_str()?;
    let new = input["new_string"].as_str()?;
    match current {
        None if old.is_empty() => Some(new.to_string()),
        None => None,
        Some(text) if old.is_empty() || !text.contains(old) => None,
        Some(text) if input["replace_all"].as_bool() == Some(true) => Some(text.replace(old, new)),
        Some(text) => Some(text.replacen(old, new, 1)),
    }
}

/// `path` with `.` dropped and `..` applied, without touching the file system.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

const BASH_ALLOWED: &str = "cargo check|build|test|clippy, cargo fmt --check, git \
    status|diff|log|show|blame|ls-files|rev-parse, piped into head|tail|grep|wc";

/// Whether an agent may run `command` through `Bash`.
pub fn bash(command: &str) -> Result<(), String> {
    let words: Vec<&str> = command.split_whitespace().collect();
    let has = |w: &str| words.iter().any(|x| x.trim_matches(['"', '\'']) == w);
    if has("git") && has("commit") {
        return Err(
            "arch: git commit is denied to agents (ADR 0016); commit through mcp__arch__commit"
                .into(),
        );
    }
    if has("git") && has("push") {
        return Err("arch: agents never push (ADR 0017); the person pushes on create PR".into());
    }
    let denied = |why: &str| {
        Err(format!(
            "arch: `{command}` is not allowed ({why}); Bash may run: {BASH_ALLOWED}"
        ))
    };
    let plain = command.replace("2>&1", "");
    if ["`", "$(", ">", "<"].iter().any(|m| plain.contains(m)) {
        return denied("no redirection or substitution");
    }
    let segments = plain
        .split("&&")
        .flat_map(|s| s.split("||"))
        .flat_map(|s| s.split([';', '|', '&', '\n']));
    for segment in segments {
        let w: Vec<&str> = segment.split_whitespace().collect();
        if w.is_empty() {
            continue;
        }
        if w.iter()
            .any(|x| x.starts_with("--config") || x.starts_with("--output") || *x == "-O")
        {
            return denied("no --config, --output or -O");
        }
        let ok = match (w[0], w.get(1).copied()) {
            ("cargo", Some("check" | "build" | "test" | "clippy")) => true,
            ("cargo", Some("fmt")) => w.contains(&"--check"),
            (
                "git",
                Some("status" | "diff" | "log" | "show" | "blame" | "ls-files" | "rev-parse"),
            ) => true,
            ("head" | "tail" | "grep" | "wc", _) => true,
            _ => false,
        };
        if !ok {
            return denied(&format!("`{}` is not on the list", segment.trim()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_allows_the_list_and_denies_the_rest() {
        for ok in [
            "cargo test -p pay",
            "cargo test 2>&1 | tail -20",
            "cargo clippy --all-targets && cargo fmt --check",
            "git status",
            "git diff HEAD~1 | grep fn",
        ] {
            assert_eq!(bash(ok), Ok(()), "{ok}");
        }
        for (no, says) in [
            ("git commit -m x", "ADR 0016"),
            ("git -C . commit --amend", "ADR 0016"),
            ("git push origin HEAD", "ADR 0017"),
            ("cargo fmt", "not on the list"),
            ("rm -rf src", "not on the list"),
            ("cargo test; curl x", "not on the list"),
            ("cargo test > out.txt", "redirection"),
            ("echo $(id)", "substitution"),
            ("git diff --output=x", "--output"),
            ("cargo --config 'target.x.runner=\"sh\"' test", "--config"),
            ("git checkout -- .", "not on the list"),
            ("cargo test & rm x", "not on the list"),
        ] {
            let reason = bash(no).expect_err(no);
            assert!(reason.contains(says), "{no}: {reason}");
        }
    }

    #[test]
    fn normalize_is_lexical() {
        assert_eq!(
            normalize(Path::new("/w/./src/../a.rs")),
            Path::new("/w/a.rs")
        );
        assert_eq!(normalize(Path::new("./src/a.rs")), Path::new("src/a.rs"));
    }
}
