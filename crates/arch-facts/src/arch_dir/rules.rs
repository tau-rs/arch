//! `rules` (spec §7): dependency rules (subject · must not · targets · level) and lint
//! settings. The template `arch init` writes is ADR 0006.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// A finding's level (spec §6, vocabulary: "blocking or not"; ADR 0009 for the confidence
/// rule that can lower it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
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

/// A lint's setting. The record has not fixed the shape (arch-design#4); both readings in
/// use are accepted: `true`/`false`, or a level `"block"` · `"warn"` · `"off"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LintSetting {
    /// On (warn) or off.
    Enabled(bool),
    /// On at a level.
    Level(LintLevel),
}

/// A lint level, when the setting names one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LintLevel {
    /// Blocks.
    Block,
    /// Warns.
    Warn,
    /// Off.
    Off,
}

impl LintSetting {
    /// Whether the lint runs.
    pub fn is_on(self) -> bool {
        !matches!(
            self,
            LintSetting::Enabled(false) | LintSetting::Level(LintLevel::Off)
        )
    }

    /// The level a finding gets: `block` only when said so, otherwise `warn`.
    pub fn level(self) -> Option<Level> {
        match self {
            LintSetting::Enabled(false) | LintSetting::Level(LintLevel::Off) => None,
            LintSetting::Level(LintLevel::Block) => Some(Level::Block),
            LintSetting::Enabled(true) | LintSetting::Level(LintLevel::Warn) => Some(Level::Warn),
        }
    }
}

/// The content of `rules`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rules {
    /// Dependency rules.
    #[serde(rename = "rule")]
    pub rules: Vec<Rule>,
    /// Lint settings by name.
    pub lints: BTreeMap<String, LintSetting>,
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
            lints: V1_LINTS
                .iter()
                .map(|l| (l.to_string(), LintSetting::Enabled(true)))
                .collect(),
        }
    }

    /// Serialize to TOML.
    pub fn to_toml(&self) -> Result<String> {
        super::to_toml(self)
    }

    /// Whether a lint is on.
    pub fn lint_on(&self, name: &str) -> bool {
        self.lints.get(name).is_some_and(|s| s.is_on())
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

    #[test]
    fn lint_settings_accept_bool_and_level() {
        let r: Rules = toml::from_str(
            "[lints]\na = true\nb = false\nc = \"block\"\nd = \"warn\"\ne = \"off\"",
        )
        .unwrap();
        assert!(
            r.lint_on("a")
                && !r.lint_on("b")
                && r.lint_on("c")
                && r.lint_on("d")
                && !r.lint_on("e")
        );
        assert!(!r.lint_on("missing"));
        assert_eq!(r.lints["c"].level(), Some(Level::Block));
        assert_eq!(r.lints["d"].level(), Some(Level::Warn));
        assert_eq!(r.lints["a"].level(), Some(Level::Warn));
        assert_eq!(r.lints["e"].level(), None);
    }
}
