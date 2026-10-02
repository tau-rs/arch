//! Sessions, plans, records and threads: what lives under `.arch/sessions/<id>/` (ADR 0003,
//! 0020, 0021) and in the session record the scheduler resumes from (ADR 0015).
//!
//! Serialized names are kebab-case; `plan.toml` and the record files are TOML, the thread is
//! JSON lines (ADR 0001: committed `.arch/` content is plain text).

mod plan;
mod record;
mod state;
mod thread;

pub use plan::*;
pub use record::*;
pub use state::*;
pub use thread::*;

/// An RFC 3339 timestamp, as written into `.arch/` files and the store.
pub type Timestamp = jiff::Timestamp;

/// The current time as a [`Timestamp`].
pub fn now() -> Timestamp {
    jiff::Timestamp::now()
}
