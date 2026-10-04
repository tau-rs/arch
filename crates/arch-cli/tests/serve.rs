//! `arch serve` as arch-app runs it: the repo as cwd, no argument, readiness = `initialize`
//! answers on the socket (ADR 0034 §2–3). And `arch --version`, the line arch-app trusts (#45).
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use arch_api::serve::{SocketEnv, socket_path};
use serde_json::Value;

fn serve(repo: &Path, runtime: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_arch"))
        .arg("serve")
        .current_dir(repo)
        .env("XDG_RUNTIME_DIR", runtime)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// Connect as the engine host does: retry every 100 ms for up to 10 s.
fn connect(path: &Path) -> UnixStream {
    let start = Instant::now();
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => return stream,
            Err(e) if start.elapsed() > Duration::from_secs(10) => {
                panic!("{}: no engine after 10 s: {e}", path.display())
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn version_prints_arch_then_semver() {
    let out = Command::new(env!("CARGO_BIN_EXE_arch"))
        .arg("--version")
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        text.lines().next(),
        Some(format!("arch {}", env!("CARGO_PKG_VERSION")).as_str())
    );
}

#[test]
fn arch_serve_answers_initialize_on_the_repo_socket_and_runs_once_per_repo() {
    let repo = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    let env = SocketEnv {
        runtime_dir: Some(runtime.path().into()),
        uid: SocketEnv::current().uid,
    };
    let path = socket_path(&repo.path().canonicalize().unwrap(), &env).unwrap();
    let _engine = Killed(serve(repo.path(), runtime.path()));

    let mut stream = connect(&path);
    stream
        .write_all(
            b"{\"jsonrpc\":\"2.0\",\"id\":\"arch-host-initialize\",\"method\":\"initialize\",\"params\":{\"client\":\"arch-app\",\"schemaVersion\":\"0.1.0\"}}\n",
        )
        .unwrap();
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["id"], "arch-host-initialize");
    assert_eq!(reply["result"]["engineVersion"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        reply["result"]["schemaVersion"],
        arch_api::rpc::API_SCHEMA_VERSION
    );

    let second = serve(repo.path(), runtime.path())
        .wait_with_output()
        .unwrap();
    assert_eq!(second.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("already serving"),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
}
