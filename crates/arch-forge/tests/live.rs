//! Opt-in, read-only run against real GitHub. Skipped unless `ARCH_FORGE_LIVE=<owner>/<repo>` is
//! set, so it never runs in default CI; it needs a token (see `resolve_token`) and never writes.
//!
//! ```sh
//! ARCH_FORGE_LIVE=tau-rs/arch cargo test -p arch-forge --test live -- --nocapture
//! ```

use arch_forge::{Forge, GitHub, RepoRef, Ureq, resolve_token};

#[test]
fn reads_a_real_repo() {
    let Ok(target) = std::env::var("ARCH_FORGE_LIVE") else {
        eprintln!("skipped: set ARCH_FORGE_LIVE=<owner>/<repo> to run against real GitHub");
        return;
    };
    let (owner, name) = target
        .split_once('/')
        .expect("ARCH_FORGE_LIVE=<owner>/<repo>");
    let token = resolve_token(|k| std::env::var(k).ok(), None).expect("a GitHub token");
    let forge = GitHub::new(RepoRef::new(owner, name), ".".into(), Ureq::new(token));

    let strategies = forge.strategies().expect("strategies");
    eprintln!("{target}: strategies {strategies:?}");
    assert_eq!(
        forge
            .pr("arch-forge-live-test/no-such-branch")
            .expect("pr lookup"),
        None
    );
}
