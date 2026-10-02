//! `arch-analyze` · watcher · cargo · git · rust-analyzer → fact deltas.
//!
//! Depends on [`arch_facts`] only. Produces [`arch_facts::FileFacts`] per changed file
//! (ADR 0002) and marks a crate that cannot be type-checked as degraded (ADR 0010).
//! Milestone 2 of `arch-design/handoffs/handoff-arch.md`; this crate is a skeleton in milestone 1.
