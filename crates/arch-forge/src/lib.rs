//! `arch-forge` · the `Forge` trait and the github adapter (ADR 0018).
//!
//! Nothing in the trait names a forge; UI words (PR · checks) come from the adapter. The
//! [`GitHub`] adapter speaks REST through a [`Transport`] port: [`Ureq`] with a token in
//! production, the [`Recorded`] double in tests and behind #48's fake forge. Pushing is
//! `git push -u origin <branch>`, on create PR or the person's click, never an agent's (ADR 0017).

mod github;
mod recorded;
mod token;
mod transport;

pub use github::{GitHub, RepoRef};
pub use recorded::Recorded;
pub use token::{Token, TokenSource, resolve_token};
pub use transport::{HttpRequest, HttpResponse, Method, Transport, Ureq};

/// Errors crossing the forge boundary.
#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    /// No token from any source (#45: env, keychain, `gh auth token`).
    #[error(
        "forge: no GitHub token: set GITHUB_TOKEN or GH_TOKEN, store one in the keychain as arch / github, or run `gh auth login`"
    )]
    NoToken,
    /// The repository's `origin` is not on GitHub.
    #[error("forge: origin is not a GitHub repository: {remote}")]
    NotGitHub {
        /// The remote URL, or why it could not be read.
        remote: String,
    },
    /// `git push` failed.
    #[error("forge: git push {branch} failed: {stderr}")]
    Push {
        /// The branch pushed.
        branch: String,
        /// What git wrote on stderr.
        stderr: String,
    },
    /// The forge answered with an error status, e.g. 405 when the merge strategy is not allowed.
    #[error("forge: {status}: {message}")]
    Rejected {
        /// The HTTP status.
        status: u16,
        /// The forge's message.
        message: String,
    },
    /// The token may not read something it needs, e.g. the merge settings without push access.
    #[error("forge: the token cannot read {what}; it needs push access to the repository")]
    NoPermission {
        /// What could not be read.
        what: String,
    },
    /// The request did not reach the forge, or no recorded answer matched it.
    #[error("forge: transport: {0}")]
    Transport(String),
    /// The forge's answer did not have the expected shape.
    #[error("forge: unexpected answer: {0}")]
    Decode(String),
}

/// A request to open: the forge's PR or MR.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewRequest {
    /// The branch to merge, pushed by `create_pr`.
    pub head: String,
    /// The branch to merge into.
    pub base: String,
    /// The title.
    pub title: String,
    /// The description.
    pub body: String,
    /// Opened as a draft.
    pub draft: bool,
}

/// A request on the forge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Its number on the forge.
    pub number: u64,
    /// Its web page.
    pub url: String,
    /// The title.
    pub title: String,
    /// The branch it merges.
    pub head: String,
    /// The branch it merges into.
    pub base: String,
    /// The head commit the forge saw: checks run on it and merge refuses if it moved.
    pub head_sha: String,
    /// A draft.
    pub draft: bool,
    /// Open, closed or merged.
    pub state: RequestState,
}

/// Where a request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestState {
    /// Open.
    Open,
    /// Closed without merging.
    Closed,
    /// Merged (a forge fact, ADR 0003).
    Merged,
}

/// One CI result on a request's head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// The check's name.
    pub name: String,
    /// Where it stands.
    pub state: CheckState,
    /// Its page on the forge or the CI.
    pub url: Option<String>,
}

/// Where a check stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    /// Queued or running.
    Pending,
    /// Succeeded.
    Passed,
    /// Failed, errored, timed out, was cancelled or needs action.
    Failed,
    /// Skipped or neutral: never blocks.
    Skipped,
}

/// The checks folded into one state: any failed → failed, else any pending → pending, else passed.
/// `None` when nothing is reported: the repo has no CI, or right after a push the forge has not
/// registered its checks yet. The caller tells the two apart.
pub fn summary(checks: &[Check]) -> Option<CheckState> {
    if checks.is_empty() {
        return None;
    }
    let any = |state| checks.iter().any(|c| c.state == state);
    Some(if any(CheckState::Failed) {
        CheckState::Failed
    } else if any(CheckState::Pending) {
        CheckState::Pending
    } else {
        CheckState::Passed
    })
}

/// A person or team asked to review, and their latest verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reviewer {
    /// The login, or `org/team` for a team.
    pub name: String,
    /// A team rather than a person.
    pub team: bool,
    /// Their verdict.
    pub state: ReviewState,
}

/// A reviewer's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    /// Asked to review, not reviewed since.
    Requested,
    /// Approved.
    Approved,
    /// Asked for changes.
    ChangesRequested,
    /// Commented only.
    Commented,
}

/// How the forge merges a request: a repo setting, read and never chosen in arch (ADR 0016).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// A merge commit.
    Merge,
    /// One squashed commit.
    Squash,
    /// The commits rebased onto the base.
    Rebase,
}

/// A merged request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    /// The commit the merge produced on the base.
    pub sha: String,
}

/// The code host behind arch (ADR 0018): GitHub first, GitLab in V1.x.
pub trait Forge {
    /// Push `branch` to `origin` and set its upstream: the person's click (ADR 0017).
    fn push(&self, branch: &str) -> Result<(), ForgeError>;
    /// Push `new.head`, then open the request: push happens on create PR (ADR 0017).
    fn create_pr(&self, new: &NewRequest) -> Result<Request, ForgeError>;
    /// The request whose head is `branch`, open, closed or merged; the open one when several.
    fn pr(&self, branch: &str) -> Result<Option<Request>, ForgeError>;
    /// The CI results on the request's head.
    fn checks(&self, pr: &Request) -> Result<Vec<Check>, ForgeError>;
    /// The reviewers asked or reviewing, with their latest verdict.
    fn reviewers(&self, pr: &Request) -> Result<Vec<Reviewer>, ForgeError>;
    /// Merge with `strategy`, refused if the head moved past `pr.head_sha`.
    fn merge(&self, pr: &Request, strategy: Strategy) -> Result<Merged, ForgeError>;
    /// The strategies the repo allows.
    fn strategies(&self) -> Result<Vec<Strategy>, ForgeError>;
    /// The forge's name for its merge request kind, for the UI ("PR", "MR").
    fn request_word(&self) -> &'static str;
    /// The forge's name for its CI runs, for the UI ("checks", "pipeline").
    fn checks_word(&self) -> &'static str;
}
