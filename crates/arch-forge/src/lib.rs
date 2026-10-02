//! `arch-forge` · the `Forge` trait and the github adapter (ADR 0018).
//!
//! Nothing in the trait names a forge; UI words (PR · checks) come from the adapter.
//! The github adapter lands with milestone 5; milestone 1 declares the boundary only.

/// A branch name on the forge.
pub type Branch = str;

/// Errors crossing the forge boundary.
#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    /// The forge rejected or failed the request.
    #[error("forge: {0}")]
    Failed(String),
}

/// The code host behind arch (ADR 0018): GitHub first, GitLab in V1.x.
///
/// Method set from `handoff-arch.md` §2; bodies arrive with milestone 5.
pub trait Forge {
    /// The forge's name for its merge request kind, for the UI ("PR", "MR").
    fn request_word(&self) -> &'static str;
    /// The forge's name for its CI runs, for the UI ("checks", "pipeline").
    fn checks_word(&self) -> &'static str;
}
