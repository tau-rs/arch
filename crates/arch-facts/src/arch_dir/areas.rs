//! `areas.toml` (ADR 0004): overrides only. Areas derive from the module tree; this file
//! holds path patterns → area, side, order, and the main bin (ADR 0007), plus the column rule
//! (spec §5: "overridable and declared in `.arch`").

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// The two column rules (spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColumnRule {
    /// driving → domain → driven → externals; chosen when the unit has an entry.
    Hexagon,
    /// public API left, internals, leaves right.
    Layers,
}

/// A side (column) an area sits in, per rule (spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Column {
    /// Hexagon: driving adapters.
    Driving,
    /// Hexagon: the domain.
    Domain,
    /// Hexagon: driven adapters.
    Driven,
    /// Hexagon: externals (the rail).
    Externals,
    /// Layers: the public API.
    PublicApi,
    /// Layers: internals.
    Internals,
    /// Layers: leaves.
    Leaves,
}

impl Column {
    /// The rule a side belongs to.
    pub fn rule(self) -> ColumnRule {
        match self {
            Column::Driving | Column::Domain | Column::Driven | Column::Externals => {
                ColumnRule::Hexagon
            }
            Column::PublicApi | Column::Internals | Column::Leaves => ColumnRule::Layers,
        }
    }
}

/// One override: path patterns → area, side, order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AreaOverride {
    /// Area name.
    pub name: String,
    /// Glob patterns over repo-relative paths.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Column, when overriding the computed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Column>,
    /// Order within the side, when overriding the computed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<u32>,
}

/// The content of `areas.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Areas {
    /// The column rule, when overriding the one chosen from the presence of an entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<ColumnRule>,
    /// The main `[[bin]]`, when overriding the first one (ADR 0007).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_bin: Option<String>,
    /// The target triple to analyse for, when pinned; this machine's otherwise (ADR 0030).
    /// `arch init` never writes it: the initialiser's triple would make every teammate on
    /// another platform cross-compile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Overrides.
    #[serde(rename = "area")]
    pub areas: Vec<AreaOverride>,
}

impl Areas {
    /// Serialize to TOML.
    pub fn to_toml(&self) -> Result<String> {
        super::to_toml(self)
    }

    /// The override for an area name.
    pub fn area(&self, name: &str) -> Option<&AreaOverride> {
        self.areas.iter().find(|a| a.name == name)
    }

    /// Set or replace an override by name (Keep writes one change, ADR 0004).
    pub fn set(&mut self, override_: AreaOverride) {
        match self.areas.iter_mut().find(|a| a.name == override_.name) {
            Some(a) => *a = override_,
            None => self.areas.push(override_),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_overrides_only() {
        let a: Areas = toml::from_str(
            r#"
rule = "hexagon"
main_bin = "smallsvc"
target = "x86_64-unknown-linux-gnu"

[[area]]
name = "ship"
paths = ["src/ship/**"]
side = "domain"
order = 2
"#,
        )
        .unwrap();
        assert_eq!(a.rule, Some(ColumnRule::Hexagon));
        assert_eq!(a.main_bin.as_deref(), Some("smallsvc"));
        assert_eq!(a.target.as_deref(), Some("x86_64-unknown-linux-gnu"));
        assert_eq!(a.area("ship").unwrap().side, Some(Column::Domain));
        let back: Areas = toml::from_str(&a.to_toml().unwrap()).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn empty_file_means_no_overrides() {
        let a: Areas = toml::from_str("").unwrap();
        assert_eq!(a, Areas::default());
    }
}
