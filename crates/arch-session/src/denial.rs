//! Telling the tool layer's denials apart (ADR 0012). Every pre-hook denial is a
//! `RecordKind::Denial`; only a write outside the element's scope is a deviation (spec §6).
//!
//! Today the only signal is the reason's wording (arch-design#117 point 2 proposes a structured
//! `cause`); this is the one place that reads it.
//!
//! | reason | cause |
//! |---|---|
//! | `arch: src/x.rs is outside element E1 (…); it may write: …` | [`Cause::Scope`] |
//! | `arch: src/x.rs exists and element E1 has not read it; read it first` | [`Cause::Stale`] |
//! | `arch: src/x.rs changed since element E1 last read it; re-read it` | [`Cause::Stale`] |
//! | `arch: git commit is denied to agents (ADR 0016)`, anything else | [`Cause::Command`] |

/// Why the tool layer refused a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// A write outside the element's files: a deviation.
    Scope,
    /// A write to a file the element has not read, or that changed since: the agent re-reads.
    Stale,
    /// A refused command (`git commit`, `git push`, a Bash command out of the allowed set).
    Command,
}

/// The cause a denial's reason states.
pub fn cause(reason: &str) -> Cause {
    if reason.contains(" is outside element ") {
        Cause::Scope
    } else if reason.contains(" has not read it") || reason.contains(" changed since element ") {
        Cause::Stale
    } else {
        Cause::Command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reason_prefixes_of_the_tool_layer() {
        let table = [
            (
                "arch: src/lib.rs is outside element E1 (a3f9c2e1); it may write: NOTES.md. If the change is needed, say why",
                Cause::Scope,
            ),
            (
                "arch: src/lib.rs exists and element E1 has not read it; read it first",
                Cause::Stale,
            ),
            (
                "arch: src/lib.rs changed since element E1 last read it; re-read it, then retry",
                Cause::Stale,
            ),
            (
                "arch: git commit is denied to agents (ADR 0016)",
                Cause::Command,
            ),
            ("arch: agents never push (ADR 0017)", Cause::Command),
            ("arch: `rm -rf .` is not an allowed command", Cause::Command),
        ];
        for (reason, expected) in table {
            assert_eq!(cause(reason), expected, "{reason}");
        }
    }
}
