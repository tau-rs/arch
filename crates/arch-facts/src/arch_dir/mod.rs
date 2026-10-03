//! Readers and writers for the committed `.arch/` files (spec §7; ADR 0001, 0003, 0004, 0021).
//!
//! ```text
//! .arch/
//!   areas.toml       overrides only (ADR 0004)
//!   areas/<name>.md  optional description per area
//!   rules            dependency rules and lint settings (TOML)
//!   allows           allowed sites, person-only (TOML)
//!   sessions/<id>/   plan.toml · thread.jsonl · records/  (ADR 0003)
//!   cache/           the sqlite store, gitignored (ADR 0001)
//!   cache/tool-layer/<element>.json  the tool layer's state, gitignored (ADR 0012)
//!   board            V1: empty
//! ```
//!
//! Everything here is plain text (ADR 0001). The `rules` and `allows` files have no extension
//! in spec §7; their content is TOML.

mod allows;
mod archive;
mod areas;
mod rules;
mod sessions;
mod tool_layer;

use std::path::{Path, PathBuf};

pub use allows::{Allow, Allows};
pub use archive::{Archive, ArchiveFile, NOTES_REF};
pub use areas::{AreaOverride, Areas, Column, ColumnRule};
pub use rules::{Level, LintLevel, LintSetting, Rule, Rules};
pub use sessions::SessionDir;
pub use tool_layer::{
    AttributedWrite, ExpectedWrite, ToolLayerFile, ToolLayerState, tool_layer_dir,
    tool_layer_states,
};

use crate::error::{Error, Result};
use crate::session::SessionId;

/// The gitignore line `arch init` adds (ADR 0001, 0006).
pub const GITIGNORE_LINE: &str = ".arch/cache/";

/// A repository's `.arch/` directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchDir {
    root: PathBuf,
}

impl ArchDir {
    /// The `.arch/` of a repository root.
    pub fn of_repo(repo_root: &Path) -> Self {
        ArchDir {
            root: repo_root.join(".arch"),
        }
    }

    /// An `.arch/` directory at an explicit path.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        ArchDir { root: root.into() }
    }

    /// The directory itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the directory exists.
    pub fn exists(&self) -> bool {
        self.root.is_dir()
    }

    /// Create the directory layout (no files): `.arch/`, `areas/`, `sessions/`, `cache/`, and
    /// an empty `board` (spec §7: V1 empty).
    pub fn create(&self) -> Result<()> {
        for d in [
            self.root.clone(),
            self.areas_dir(),
            self.sessions_dir(),
            self.cache_dir(),
        ] {
            std::fs::create_dir_all(&d).map_err(|e| Error::io(&d, e))?;
        }
        let board = self.board_path();
        if !board.exists() {
            std::fs::write(&board, "").map_err(|e| Error::io(&board, e))?;
        }
        Ok(())
    }

    /// `.arch/areas.toml`.
    pub fn areas_path(&self) -> PathBuf {
        self.root.join("areas.toml")
    }
    /// `.arch/areas/`.
    pub fn areas_dir(&self) -> PathBuf {
        self.root.join("areas")
    }
    /// `.arch/rules`.
    pub fn rules_path(&self) -> PathBuf {
        self.root.join("rules")
    }
    /// `.arch/allows`.
    pub fn allows_path(&self) -> PathBuf {
        self.root.join("allows")
    }
    /// `.arch/sessions/`.
    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }
    /// `.arch/cache/`.
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }
    /// `.arch/board`.
    pub fn board_path(&self) -> PathBuf {
        self.root.join("board")
    }

    /// Read `areas.toml`; absent means no overrides (ADR 0004).
    pub fn read_areas(&self) -> Result<Areas> {
        read_toml_or_default(&self.areas_path())
    }

    /// Write `areas.toml` (the one write Keep makes, ADR 0004).
    pub fn write_areas(&self, areas: &Areas) -> Result<()> {
        write_text(&self.areas_path(), &areas.to_toml()?)
    }

    /// The description of an area, from `areas/<name>.md`, read into the context pack.
    pub fn area_description(&self, name: &str) -> Result<Option<String>> {
        let p = self.areas_dir().join(format!("{name}.md"));
        match std::fs::read_to_string(&p) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(p, e)),
        }
    }

    /// Write an area's description.
    pub fn write_area_description(&self, name: &str, text: &str) -> Result<()> {
        let dir = self.areas_dir();
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        write_text(&dir.join(format!("{name}.md")), text)
    }

    /// Read `rules`; absent means no rules and no lints.
    pub fn read_rules(&self) -> Result<Rules> {
        read_toml_or_default(&self.rules_path())
    }

    /// Write `rules`.
    pub fn write_rules(&self, rules: &Rules) -> Result<()> {
        write_text(&self.rules_path(), &rules.to_toml()?)
    }

    /// Read `allows`; absent means none.
    pub fn read_allows(&self) -> Result<Allows> {
        read_toml_or_default(&self.allows_path())
    }

    /// Write `allows`.
    pub fn write_allows(&self, allows: &Allows) -> Result<()> {
        write_text(&self.allows_path(), &allows.to_toml()?)
    }

    /// A session's folder, `sessions/<id>/` (ADR 0003). Not created until written to.
    pub fn session(&self, id: &SessionId) -> SessionDir {
        SessionDir::new(self.sessions_dir().join(id.as_str()), id.clone())
    }

    /// The sessions present on this branch, by folder name.
    pub fn sessions(&self) -> Result<Vec<SessionId>> {
        let dir = self.sessions_dir();
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(Error::io(&dir, e)),
        };
        let mut ids = vec![];
        for entry in rd {
            let entry = entry.map_err(|e| Error::io(&dir, e))?;
            if entry.path().is_dir() {
                ids.push(SessionId::new(
                    entry.file_name().to_string_lossy().to_string(),
                ));
            }
        }
        ids.sort();
        Ok(ids)
    }
}

fn read_toml_or_default<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s).map_err(|e| Error::format(path, e.message())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(Error::io(path, e)),
    }
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    std::fs::write(path, text).map_err(|e| Error::io(path, e))
}

fn to_toml<T: serde::Serialize>(value: &T) -> Result<String> {
    toml::to_string_pretty(value).map_err(|e| Error::Other(anyhow::anyhow!("toml: {e}")))
}
