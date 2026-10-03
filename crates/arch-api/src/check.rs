//! `arch check`: facts, findings, a versioned output document, an exit code.
//!
//! The whole tree is checked; scoping the findings to a diff comes with the store-backed path.

use std::path::Path;

use arch_analyze::{Commits, Options, analyze};
use arch_facts::{ArchDir, Degraded};
use arch_views::{Finding, check_rules};
use schemars::JsonSchema;
use serde::Serialize;

use crate::Error;

/// Version of the [`CheckOutput`] shape, published as `schemas/check.schema.json`.
pub const CHECK_SCHEMA_VERSION: u32 = 0;

/// What `arch check --format json` prints.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CheckOutput {
    /// Shape version of this document.
    pub schema_version: u32,
    /// Repository name.
    pub repo: String,
    /// The commit (or `wt:` worktree-state hash) the facts describe.
    pub commit: String,
    /// The analyzer and its version.
    pub analyzer: String,
    /// Crates analyzed at syntax level only: their facts are guessed, so findings on them warn
    /// and never block (ADR 0009, 0010).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub degraded: Vec<Degraded>,
    /// Counts.
    pub summary: Summary,
    /// Every finding, allowed ones included, sorted by site, target and rule.
    pub findings: Vec<Finding>,
}

/// Finding counts by outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Summary {
    /// Findings that fail the check.
    pub blocking: usize,
    /// Findings that warn.
    pub warnings: usize,
    /// Findings covered by `.arch/allows`.
    pub allowed: usize,
}

impl CheckOutput {
    /// The process exit code: 0 when nothing blocks, 1 otherwise. (2 is a tool error.)
    pub fn exit_code(&self) -> u8 {
        u8::from(self.summary.blocking > 0)
    }

    /// The JSON Schema (draft 2020-12) of this document, as published in `schemas/check.schema.json`.
    pub fn json_schema() -> serde_json::Value {
        let mut schema = schemars::schema_for!(CheckOutput);
        schema.insert(
            "$id".into(),
            "https://github.com/tau-rs/arch/blob/main/schemas/check.schema.json".into(),
        );
        schema.into()
    }
}

/// Check the repository at `repo` against its `.arch/rules`.
pub fn check(repo: &Path) -> Result<CheckOutput, Error> {
    let arch = ArchDir::of_repo(repo);
    if !arch.areas_path().is_file() && !arch.rules_path().is_file() {
        return Err(Error::NotInitialized(repo.to_path_buf()));
    }
    let options = Options {
        commits: Commits::None,
        ..Options::default()
    };
    let facts = analyze(repo, &options)?;
    let report = check_rules(
        &facts,
        &arch.read_areas()?,
        &arch.read_rules()?,
        &arch.read_allows()?,
    )?;
    Ok(CheckOutput {
        schema_version: CHECK_SCHEMA_VERSION,
        repo: facts.repo.name,
        commit: facts.repo.commit,
        analyzer: format!("{} {}", facts.analyzer.name, facts.analyzer.version),
        degraded: facts.analyzer.degraded,
        summary: Summary {
            blocking: report.blocking().count(),
            warnings: report.warnings().count(),
            allowed: report.allowed().count(),
        },
        findings: report.findings,
    })
}
