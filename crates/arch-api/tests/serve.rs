//! `arch serve`: the per-repo socket, the JSON-RPC framing, `initialize` (ADR 0034 §2–3).
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use arch_api::rpc::{API_SCHEMA_VERSION, handle_line};
use arch_api::serve::{ServeError, SocketEnv, bind, hash8, socket_path};
use serde_json::{Value, json};

fn reply(line: &str) -> Value {
    serde_json::from_str(&handle_line(line).expect("a request gets a reply")).unwrap()
}

#[test]
fn initialize_answers_the_engine_and_schema_versions() {
    let r = reply(
        r#"{"jsonrpc":"2.0","id":"arch-host-initialize","method":"initialize","params":{"client":"arch-app","schemaVersion":"none"}}"#,
    );
    assert_eq!(
        r,
        json!({
            "jsonrpc": "2.0",
            "id": "arch-host-initialize",
            "result": { "engineVersion": env!("CARGO_PKG_VERSION"), "schemaVersion": API_SCHEMA_VERSION },
        })
    );
}

#[test]
fn schema_version_is_optional_and_a_numeric_id_comes_back() {
    let r = reply(r#"{"jsonrpc":"2.0","id":7,"method":"initialize","params":{"client":"t"}}"#);
    assert_eq!(r["id"], 7);
    assert_eq!(r["result"]["schemaVersion"], API_SCHEMA_VERSION);
}

#[test]
fn json_rpc_errors_use_the_standard_codes() {
    let code = |line: &str| reply(line)["error"]["code"].as_i64().unwrap();
    assert_eq!(code("{not json"), -32700);
    assert_eq!(
        code(r#"[{"jsonrpc":"2.0","id":1,"method":"initialize"}]"#),
        -32600
    );
    assert_eq!(
        code(r#"{"jsonrpc":"1.0","id":1,"method":"initialize"}"#),
        -32600
    );
    assert_eq!(
        code(r#"{"jsonrpc":"2.0","id":1,"method":"views.get"}"#),
        -32601
    );
    assert_eq!(
        code(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#),
        -32602
    );
    assert_eq!(
        code(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"client":3}}"#),
        -32602
    );
}

#[test]
fn a_notification_and_a_blank_line_get_no_reply() {
    assert_eq!(
        handle_line(r#"{"jsonrpc":"2.0","method":"initialize","params":{"client":"t"}}"#),
        None
    );
    assert_eq!(handle_line("   "), None);
}

/// arch-app's `hash8` (FNV-1a 32 over UTF-16 code units); values computed with its code.
#[test]
fn hash8_matches_arch_app() {
    assert_eq!(hash8("/repo"), "81f9fa62");
    assert_eq!(hash8("/Users/me/code/orderly"), "0366f73b");
    assert_eq!(hash8("/tmp/café"), "4eabacfd");
}

#[test]
fn the_socket_path_follows_arch_app_s_convention() {
    let runtime = SocketEnv {
        runtime_dir: Some("/run/user/1000/".into()),
        uid: 1000,
    };
    let no_runtime = SocketEnv {
        runtime_dir: None,
        uid: 501,
    };
    assert_eq!(
        socket_path("/repo/".as_ref(), &runtime).unwrap(),
        PathBuf::from("/run/user/1000/arch/81f9fa62.sock")
    );
    assert_eq!(
        socket_path("/repo".as_ref(), &no_runtime).unwrap(),
        PathBuf::from("/tmp/arch-501/81f9fa62.sock")
    );
    let empty = SocketEnv {
        runtime_dir: Some("".into()),
        uid: 501,
    };
    assert_eq!(
        socket_path("/repo".as_ref(), &empty).unwrap(),
        PathBuf::from("/tmp/arch-501/81f9fa62.sock")
    );
}

/// A server on a scratch runtime dir, run on its own thread.
struct Running {
    path: PathBuf,
    _runtime: tempfile::TempDir,
    _repo: tempfile::TempDir,
}

fn env_in(runtime: &Path) -> SocketEnv {
    SocketEnv {
        runtime_dir: Some(runtime.into()),
        uid: SocketEnv::current().uid,
    }
}

fn start() -> Running {
    let runtime = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let server = bind(repo.path(), &env_in(runtime.path())).unwrap();
    let path = server.path().to_path_buf();
    std::thread::spawn(move || server.run());
    Running {
        path,
        _runtime: runtime,
        _repo: repo,
    }
}

fn call(stream: &mut UnixStream, reader: &mut impl BufRead, line: &str) -> Value {
    stream.write_all(line.as_bytes()).unwrap();
    stream.write_all(b"\n").unwrap();
    let mut answer = String::new();
    reader.read_line(&mut answer).unwrap();
    serde_json::from_str(&answer).unwrap()
}

#[test]
fn a_client_initializes_over_the_socket_and_keeps_the_connection() {
    let running = start();
    let mut stream = UnixStream::connect(&running.path).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"client":"t"}}"#;
    assert_eq!(
        call(&mut stream, &mut reader, init)["result"]["schemaVersion"],
        API_SCHEMA_VERSION
    );
    let again = call(
        &mut stream,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":2,"method":"nope"}"#,
    );
    assert_eq!(again["id"], 2);
    assert_eq!(again["error"]["code"], -32601);
    // a second client is served alongside the first
    let mut other = UnixStream::connect(&running.path).unwrap();
    let mut other_reader = BufReader::new(other.try_clone().unwrap());
    assert_eq!(call(&mut other, &mut other_reader, init)["id"], 1);
}

#[test]
fn the_socket_directory_is_private() {
    let running = start();
    let mode = std::fs::metadata(running.path.parent().unwrap())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[test]
fn a_second_engine_on_the_same_repo_refuses_to_start() {
    let runtime = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let first = bind(repo.path(), &env_in(runtime.path())).unwrap();
    std::thread::spawn(move || first.run());
    assert!(matches!(
        bind(repo.path(), &env_in(runtime.path())),
        Err(ServeError::AlreadyServing(_))
    ));
}

#[test]
fn a_stale_socket_file_is_replaced() {
    let runtime = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let env = env_in(runtime.path());
    // a bound-then-dropped listener leaves a file nobody accepts on
    drop(bind(repo.path(), &env).unwrap());
    let server = bind(repo.path(), &env).unwrap();
    assert!(server.path().exists());
}

#[test]
fn a_socket_directory_open_to_others_is_refused() {
    let runtime = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let dir = runtime.path().join("arch");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        bind(repo.path(), &env_in(runtime.path())),
        Err(ServeError::UnsafeDir { .. })
    ));
}

#[test]
fn the_repo_root_is_hashed_through_its_real_path() {
    let runtime = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let link_parent = tempfile::tempdir().unwrap();
    let link = link_parent.path().join("via-link");
    std::os::unix::fs::symlink(repo.path(), &link).unwrap();
    let env = env_in(runtime.path());
    let real = repo.path().canonicalize().unwrap();
    let expected = socket_path(&real, &env).unwrap();
    assert_eq!(bind(&link, &env).unwrap().path(), expected);
}
