//! `allows` (spec §7): allowed sites, keyed by site, person-only.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::session::Timestamp;

/// One allow: a site where a rule or lint does not fire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allow {
    /// The site: an item id, or `file:line`.
    pub site: String,
    /// The rule (`subject must not ... targets`) or lint name allowed there.
    pub rule: String,
    /// The link's target the allow is for, when the finding is on a link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Why.
    pub reason: String,
    /// Who (a person; agents cannot write this file).
    pub by: String,
    /// When, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Timestamp>,
}

/// The content of `allows`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Allows {
    /// Allows.
    #[serde(rename = "allow")]
    pub allows: Vec<Allow>,
}

impl Allows {
    /// Serialize to TOML.
    pub fn to_toml(&self) -> Result<String> {
        super::to_toml(self)
    }

    /// Whether a rule is allowed at a site.
    pub fn allows(&self, site: &str, rule: &str) -> bool {
        self.allows.iter().any(|a| a.site == site && a.rule == rule)
    }

    /// The allows at a site.
    pub fn at(&self, site: &str) -> impl Iterator<Item = &Allow> {
        self.allows.iter().filter(move |a| a.site == site)
    }

    /// Add one entry (spec §6 Daily: "allow is person-only, one entry in `.arch/allows`").
    pub fn add(&mut self, allow: Allow) {
        self.allows.push(allow);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_by_site() {
        let mut a = Allows::default();
        a.add(Allow {
            site: "src/store/pg.rs::dequeue".into(),
            rule: "domain must not depend-on driven".into(),
            target: None,
            reason: "transitional".into(),
            by: "titouan".into(),
            at: Some(crate::session::now()),
        });
        assert!(a.allows(
            "src/store/pg.rs::dequeue",
            "domain must not depend-on driven"
        ));
        assert!(!a.allows("src/store/pg.rs::dequeue", "cycle"));
        let back: Allows = toml::from_str(&a.to_toml().unwrap()).unwrap();
        assert_eq!(a, back);
    }
}
