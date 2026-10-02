//! Plans and elements (spec §6, §7; ADR 0020, 0021).

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::SessionId;

/// An element's id (ADR 0021): `sha256(session · intention · site)[:8]`.
///
/// The hash input is the UTF-8 of the three strings joined by NUL; see
/// arch-design#18 (via `docs/arch-facts.md`) for the question to the product chat on this
/// encoding.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ElementId(String);

impl ElementId {
    /// Derive the id from its three parts (ADR 0021).
    pub fn derive(session: &SessionId, intention: &str, site: &str) -> Self {
        let mut h = Sha256::new();
        h.update(session.as_str().as_bytes());
        h.update([0u8]);
        h.update(intention.as_bytes());
        h.update([0u8]);
        h.update(site.as_bytes());
        ElementId(hex::encode(h.finalize())[..8].to_string())
    }

    /// Wrap an id read from a trailer or a file, without re-deriving it.
    pub fn from_str_unchecked(s: &str) -> Self {
        ElementId(s.to_string())
    }

    /// The 8 hex characters.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where an element stands (spec §6 Session, Sync).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ElementState {
    /// Planned, not started.
    #[default]
    Planned,
    /// A sub-agent is on it.
    Running,
    /// Realized.
    Done,
    /// Its site changed under it (reconcile): re-plan, drop, or revert.
    Stale,
    /// Taken over by hand.
    TakenOver,
    /// Dropped at reconcile.
    Dropped,
}

/// One planned change: an intention and a site (spec §6; ADR 0021).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Element {
    /// Content-addressed id.
    pub id: ElementId,
    /// Display label, `E3`.
    pub label: String,
    /// What to do.
    pub intention: String,
    /// Where: an item id, a file, an area.
    pub site: String,
    /// The files the element's sub-agent may write (the core veto, spec §8).
    #[serde(default)]
    pub files: Vec<std::path::PathBuf>,
    /// Elements this one depends on.
    #[serde(default)]
    pub depends_on: Vec<ElementId>,
    /// State.
    #[serde(default)]
    pub state: ElementState,
    /// A resolve element for a conflict (ADR 0019).
    #[serde(default)]
    pub resolve: bool,
}

/// What runs when a group ends (spec §6, §8): commands, `arch check`, the judge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gate {
    /// The project's test command and any other commands.
    #[serde(default)]
    pub commands: Vec<String>,
    /// Run `arch check`.
    #[serde(default = "default_true")]
    pub check: bool,
    /// Run the judge (ADR 0013).
    #[serde(default = "default_true")]
    pub judge: bool,
    /// Fix-round budget before the four-door question (spec §6: 2).
    #[serde(default = "default_fix_rounds")]
    pub fix_rounds: u8,
}

fn default_true() -> bool {
    true
}
fn default_fix_rounds() -> u8 {
    2
}

impl Default for Gate {
    fn default() -> Self {
        Gate {
            commands: vec![],
            check: true,
            judge: true,
            fix_rounds: 2,
        }
    }
}

/// Elements grouped by dependency, one gate per group (spec §3, §8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// Name, a lane on the session card.
    pub name: String,
    /// Member elements, in order.
    pub elements: Vec<ElementId>,
    /// The gate.
    #[serde(default)]
    pub gate: Gate,
}

/// A plan: a session that has not run (spec §6); `plan.toml` owns the elements (ADR 0021).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// The session.
    pub session: SessionId,
    /// The intention the plan was drafted from.
    pub intention: String,
    /// When it was drafted.
    pub created: super::Timestamp,
    /// Elements.
    #[serde(default)]
    pub elements: Vec<Element>,
    /// Groups.
    #[serde(default)]
    pub groups: Vec<Group>,
}

impl Plan {
    /// A new, empty plan for a session.
    pub fn new(session: SessionId, intention: impl Into<String>) -> Self {
        Plan {
            session,
            intention: intention.into(),
            created: super::now(),
            elements: vec![],
            groups: vec![],
        }
    }

    /// Add an element; the id is derived (ADR 0021), the label is `E<n>` by position.
    pub fn add_element(
        &mut self,
        intention: impl Into<String>,
        site: impl Into<String>,
    ) -> &Element {
        let intention = intention.into();
        let site = site.into();
        let id = ElementId::derive(&self.session, &intention, &site);
        let label = format!("E{}", self.elements.len() + 1);
        self.elements.push(Element {
            id,
            label,
            intention,
            site,
            files: vec![],
            depends_on: vec![],
            state: ElementState::Planned,
            resolve: false,
        });
        self.elements.last().expect("just pushed")
    }

    /// The element with this id.
    pub fn element(&self, id: &ElementId) -> Option<&Element> {
        self.elements.iter().find(|e| &e.id == id)
    }

    /// The element at this site, for continuity by site (ADR 0021).
    pub fn element_at(&self, site: &str) -> Option<&Element> {
        self.elements.iter().find(|e| e.site == site)
    }

    /// The group an element belongs to.
    pub fn group_of(&self, id: &ElementId) -> Option<&Group> {
        self.groups.iter().find(|g| g.elements.contains(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_id_is_eight_hex_chars_and_content_addressed() {
        let s = SessionId::new("s1");
        let a = ElementId::derive(&s, "add a port", "src/ship.rs");
        assert_eq!(a.as_str().len(), 8);
        assert!(a.as_str().chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(a, ElementId::derive(&s, "add a port", "src/ship.rs"));
        assert_ne!(a, ElementId::derive(&s, "add a port", "src/pay.rs"));
        assert_ne!(
            a,
            ElementId::derive(&SessionId::new("s2"), "add a port", "src/ship.rs")
        );
    }

    #[test]
    fn labels_follow_position() {
        let mut p = Plan::new(SessionId::new("s1"), "ship it");
        p.add_element("a", "x");
        let e2 = p.add_element("b", "y").clone();
        assert_eq!(e2.label, "E2");
        assert_eq!(p.element_at("y"), Some(&e2));
    }
}
