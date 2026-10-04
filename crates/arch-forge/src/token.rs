//! The GitHub token: `GITHUB_TOKEN`, `GH_TOKEN`, the OS keychain entry `arch` / `github`, then
//! `gh auth token`. It lives in memory only, never under `.arch/` (ADR 0023).

use std::ffi::OsStr;
use std::fmt;
use std::process::{Command, Stdio};

use crate::ForgeError;

/// Where a token came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    /// An environment variable.
    Env(&'static str),
    /// The OS keychain entry `arch` / `github`.
    Keychain,
    /// `gh auth token`.
    Gh,
}

/// A forge token. Its `Debug` never shows the secret.
#[derive(Clone)]
pub struct Token {
    secret: String,
    source: TokenSource,
}

impl Token {
    /// The secret, for the `Authorization` header only.
    pub fn secret(&self) -> &str {
        &self.secret
    }

    /// Where it came from.
    pub fn source(&self) -> TokenSource {
        self.source
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

/// The keychain lookup for this OS: `security` on macOS, `secret-tool` (libsecret) on Linux.
const KEYCHAIN: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
    &[(
        "security",
        &["find-generic-password", "-s", "arch", "-a", "github", "-w"],
    )]
} else if cfg!(target_os = "linux") {
    &[(
        "secret-tool",
        &["lookup", "service", "arch", "account", "github"],
    )]
} else {
    &[]
};

/// Resolve the token in order. `env` reads a variable; `path` is the `PATH` the keychain tool and
/// `gh` are looked up on (`None`: this process's). A source that is unset, empty, missing or
/// fails gives way to the next.
pub fn resolve_token(
    env: impl Fn(&str) -> Option<String>,
    path: Option<&OsStr>,
) -> Result<Token, ForgeError> {
    let found = |secret: Option<String>, source| {
        secret
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|secret| Token { secret, source })
    };
    for var in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Some(token) = found(env(var), TokenSource::Env(var)) {
            return Ok(token);
        }
    }
    for (program, args) in KEYCHAIN {
        if let Some(token) = found(run(program, args, path), TokenSource::Keychain) {
            return Ok(token);
        }
    }
    found(run("gh", &["auth", "token"], path), TokenSource::Gh).ok_or(ForgeError::NoToken)
}

/// The program's stdout when it exits 0.
fn run(program: &str, args: &[&str], path: Option<&OsStr>) -> Option<String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let out = command.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}
