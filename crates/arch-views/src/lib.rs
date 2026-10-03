//! `arch-views` · pure functions over the store, cached per branch.
//!
//! Column rules, positions, fold, Reach, the four overlays, findings, impact, checklist,
//! What's new, the review view. Depends on [`arch_facts`] only; no analyzer in this crate.
//! Milestone 3 of `arch-design/handoffs/handoff-arch.md`.
//!
//! Present today, for `arch check` (milestone 4, issue #5): [`placement`] (which area and side
//! an item sits in) and [`findings`] (dependency rules evaluated on links).

pub mod findings;
pub mod placement;

pub use findings::{Allowed, Finding, Report, check_rules};
pub use placement::{Placement, Placements};

/// Errors crossing this crate's boundary.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A path pattern in `areas.toml` is not a valid glob.
    #[error("areas.toml: area `{area}`: invalid path pattern `{pattern}`: {reason}")]
    BadPattern {
        /// The area whose pattern is invalid.
        area: String,
        /// The pattern.
        pattern: String,
        /// Why.
        reason: String,
    },
}
