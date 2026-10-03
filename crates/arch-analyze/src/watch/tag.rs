//! Who wrote a file. A file-system event names no process, so the tool layer (ADR 0012) records
//! the writes it is about to make, by path and content hash, and the watcher matches the file it
//! sees against them: a match is the session's write, anything else is `you` (spec §4 Work by
//! hand). The recorded content, not the path alone, decides: your edit right after an agent's
//! write to the same file is still yours.
//!
//! Provisional reading of tau-rs/arch-design#34, until arch-driver (tau-rs/arch#46) owns the
//! format: every `<worktree>/.arch/cache/tool-layer/*.json` is
//!
//! ```json
//! { "session": "s-…", "element": "e-…", "expected": [ { "path": "src/pay.rs", "sha256": "…" } ] }
//! ```

use std::path::{Path, PathBuf};

use arch_facts::{Attribution, ContentHash, ElementId, SessionId};
use serde::Deserialize;

/// Where the tool layer keeps its state, relative to a worktree.
const TOOL_LAYER: &str = ".arch/cache/tool-layer";

#[derive(Deserialize)]
struct Registry {
    session: SessionId,
    #[serde(default)]
    element: Option<ElementId>,
    #[serde(default)]
    expected: Vec<Expected>,
}

#[derive(Deserialize)]
struct Expected {
    path: PathBuf,
    sha256: String,
}

/// Who wrote `rel` (relative to `worktree`) with content `hash`: the session whose tool layer
/// expects exactly that content, or `you`. A registry file that cannot be read claims nothing.
pub fn attribute(worktree: &Path, rel: &Path, hash: &ContentHash) -> Attribution {
    let Ok(entries) = std::fs::read_dir(worktree.join(TOOL_LAYER)) else {
        return Attribution::You;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    for file in files {
        let Some(registry) = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str::<Registry>(&text).ok())
        else {
            continue;
        };
        if registry
            .expected
            .iter()
            .any(|e| e.path == rel && e.sha256 == hash.0)
        {
            return Attribution::Session {
                session: registry.session,
                element: registry.element,
            };
        }
    }
    Attribution::You
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(dir: &Path, name: &str, json: &str) {
        let d = dir.join(TOOL_LAYER);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(name), json).unwrap();
    }

    #[test]
    fn the_session_expecting_the_content_wrote_it_and_anything_else_is_you() {
        let tmp = tempfile::tempdir().unwrap();
        let h = ContentHash::of_str("agent");
        let rel = Path::new("src/pay.rs");
        assert_eq!(attribute(tmp.path(), rel, &h), Attribution::You);

        registry(tmp.path(), "broken.json", "{ not json");
        registry(tmp.path(), "notes.txt", "ignored");
        registry(
            tmp.path(),
            "e-1.json",
            &format!(
                r#"{{"session":"s-1","element":"e-1","expected":[{{"path":"src/pay.rs","sha256":"{}"}}]}}"#,
                h.0
            ),
        );
        let Attribution::Session { session, element } = attribute(tmp.path(), rel, &h) else {
            panic!("the expected write is the session's");
        };
        assert_eq!(session.as_str(), "s-1");
        assert_eq!(element.unwrap().as_str(), "e-1");

        let other = ContentHash::of_str("yours");
        assert_eq!(attribute(tmp.path(), rel, &other), Attribution::You);
        assert_eq!(
            attribute(tmp.path(), Path::new("src/other.rs"), &h),
            Attribution::You
        );
    }
}
