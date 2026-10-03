//! Print a repository's facts as JSON, the way a golden `facts.json` is generated:
//!
//! ```text
//! cargo run -p arch-analyze --example emit-facts -- <repo> [--name <name>] [--commit <hash>] [--no-commits]
//! ```
use std::path::PathBuf;

use arch_analyze::{Commits, Options, analyze};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut repo = None;
    let mut options = Options::default();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--name" => options.repo_name = args.next(),
            "--commit" => options.commit = args.next(),
            "--no-commits" => options.commits = Commits::None,
            _ => repo = Some(PathBuf::from(a)),
        }
    }
    let repo = repo.ok_or_else(|| {
        anyhow::anyhow!("usage: emit-facts <repo> [--name <name>] [--commit <hash>] [--no-commits]")
    })?;
    let facts = analyze(&repo, &options)?;
    println!("{}", serde_json::to_string_pretty(&facts)?);
    Ok(())
}
