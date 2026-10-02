//! `rules` (spec §7): dependency rules (subject · must not · targets · level) and lint
//! settings. The template `arch init` writes is ADR 0006.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// A finding's level (spec §6, vocabulary: "blocking or not"; ADR 0009 for the confidence
/// rule that can lower it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    /// Blocks a gate or a merge.
    Block,
    /// Warns.
    Warn,
}

/// A dependency rule: `subject must not <verb> targets`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// The subject: a side or an area name.
    pub subject: String,
    /// The forbidden relation, `depend-on` in V1.
    pub must_not: String,
    /// The targets: sides or area names.
    pub targets: Vec<String>,
    /// Level.
    pub level: Level,
}

/// The five lints `arch init` turns on (flow pages `daily-shell.html`, `map-focus.html`).
pub const V1_LINTS: [&str; 5] = [
    "god-module",
    "cycle",
    "leaky-port",
    "speculative-abstraction",
    "unresolved-dyn",
];

/// The content of `rules`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rules {
    /// Dependency rules.
    #[serde(rename = "rule")]
    pub rules: Vec<Rule>,
    /// Lint settings: name → on.
    pub lints: BTreeMap<String, bool>,
}

impl Rules {
    /// The template of ADR 0006: domain must not depend on driving or driven; externals only
    /// from driven; the five lints on.
    pub fn v1_template() -> Self {
        Rules {
            rules: vec![
                Rule {
                    subject: "domain".into(),
                    must_not: "depend-on".into(),
                    targets: vec!["driving".into(), "driven".into()],
                    level: Level::Block,
                },
                Rule {
                    subject: "driving".into(),
                    must_not: "depend-on".into(),
                    targets: vec!["externals".into()],
                    level: Level::Block,
                },
                Rule {
                    subject: "domain".into(),
                    must_not: "depend-on".into(),
                    targets: vec!["externals".into()],
                    level: Level::Block,
                },
            ],
            lints: V1_LINTS.iter().map(|l| (l.to_string(), true)).collect(),
        }
    }

    /// Serialize to TOML.
    pub fn to_toml(&self) -> Result<String> {
        super::to_toml(self)
    }

    /// Whether a lint is on.
    pub fn lint_on(&self, name: &str) -> bool {
        self.lints.get(name).copied().unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_round_trips_and_has_five_lints_on() {
        let t = Rules::v1_template();
        assert_eq!(t.lints.len(), 5);
        assert!(V1_LINTS.iter().all(|l| t.lint_on(l)));
        let back: Rules = toml::from_str(&t.to_toml().unwrap()).unwrap();
        assert_eq!(t, back);
    }
}
