//! The fact model: what `arch-analyze` produces, what the store keeps, what `arch-views` reads.
//!
//! This crate depends on nothing else of ours. It owns `schemas/facts.schema.json`, generated from
//! the types in [`model`] (`ARCH_UPDATE_SCHEMA=1 cargo test -p arch-facts` regenerates it; CI fails
//! when the committed file drifts). Design notes: `docs/arch-facts.md`.
//!
//! Spec references: handoff-arch.md §2 arch-facts; spec §7; ADR 0001, 0002, 0009, 0021.

pub mod model;

pub use model::*;
