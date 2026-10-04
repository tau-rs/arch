//! Token resolution order (#45): `GITHUB_TOKEN`, `GH_TOKEN`, the keychain entry `arch` / `github`,
//! then `gh auth token`. The keychain tool and `gh` are fake scripts on a temp `PATH`.

#![cfg(unix)]

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::OnceLock;

use arch_forge::{ForgeError, TokenSource, resolve_token};
use pretty_assertions::assert_eq;

fn script(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A `PATH` holding a fake `gh` and fake keychain tools (`security` on macOS, `secret-tool` on
/// Linux), each answering only for the exact arguments arch must pass.
fn write_fakes(dir: &Path, keychain: bool, gh: bool) {
    if keychain {
        script(
            dir,
            "security",
            r#"[ "$*" = "find-generic-password -s arch -a github -w" ] && echo from-keychain && exit 0; exit 44"#,
        );
        script(
            dir,
            "secret-tool",
            r#"[ "$*" = "lookup service arch account github" ] && echo from-keychain && exit 0; exit 1"#,
        );
    }
    if gh {
        script(
            dir,
            "gh",
            r#"[ "$*" = "auth token" ] && echo ' from-gh ' && exit 0; exit 1"#,
        );
    }
}

/// Every fake `PATH`, written once before any test spawns: on Linux, a script still open for
/// writing in one test thread's fork fails another thread's exec with ETXTBSY.
fn fake_path(keychain: bool, gh: bool) -> &'static Path {
    static DIRS: OnceLock<Vec<(bool, bool, tempfile::TempDir)>> = OnceLock::new();
    let dirs = DIRS.get_or_init(|| {
        [(true, true), (false, true), (false, false)]
            .into_iter()
            .map(|(k, g)| {
                let dir = tempfile::tempdir().unwrap();
                write_fakes(dir.path(), k, g);
                (k, g, dir)
            })
            .collect()
    });
    let (_, _, dir) = dirs
        .iter()
        .find(|(k, g, _)| (*k, *g) == (keychain, gh))
        .unwrap();
    dir.path()
}

fn resolve(env: &[(&str, &str)], path: &Path) -> Result<(String, TokenSource), ForgeError> {
    let env: HashMap<String, String> = env
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let token = resolve_token(|k| env.get(k).cloned(), Some(path.as_os_str()))?;
    Ok((token.secret().to_string(), token.source()))
}

#[test]
fn github_token_comes_first() {
    let path = fake_path(true, true);
    let got = resolve(&[("GITHUB_TOKEN", "a"), ("GH_TOKEN", "b")], path).unwrap();
    assert_eq!(got, ("a".into(), TokenSource::Env("GITHUB_TOKEN")));
}

#[test]
fn gh_token_comes_second() {
    let path = fake_path(true, true);
    let got = resolve(&[("GITHUB_TOKEN", ""), ("GH_TOKEN", "b")], path).unwrap();
    assert_eq!(got, ("b".into(), TokenSource::Env("GH_TOKEN")));
}

#[test]
fn the_keychain_comes_before_gh() {
    let path = fake_path(true, true);
    assert_eq!(
        resolve(&[], path).unwrap(),
        ("from-keychain".into(), TokenSource::Keychain)
    );
}

#[test]
fn gh_auth_token_comes_last() {
    let path = fake_path(false, true);
    assert_eq!(
        resolve(&[], path).unwrap(),
        ("from-gh".into(), TokenSource::Gh)
    );
}

#[test]
fn no_source_at_all_is_no_token() {
    let path = fake_path(false, false);
    assert!(matches!(resolve(&[], path), Err(ForgeError::NoToken)));
}

#[test]
fn the_token_never_shows_in_debug_output() {
    let path = fake_path(false, false);
    let token = resolve_token(
        |k| (k == "GH_TOKEN").then(|| "ghp_secret".to_string()),
        Some(path.as_os_str()),
    )
    .unwrap();
    assert!(!format!("{token:?}").contains("ghp_secret"));
}
