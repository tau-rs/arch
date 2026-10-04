//! `arch serve`: the app method set on a per-repo unix socket, one JSON-RPC 2.0 message per line
//! (ADR 0034 §2).
//!
//! The path is derived from the repo root alone, so arch-app finds it without being told:
//! `$XDG_RUNTIME_DIR/arch/<hash8>.sock`, or `/tmp/arch-<uid>/<hash8>.sock` without a runtime dir.
//! One engine per repo: a socket that answers means one is running; one that does not is stale
//! and is replaced. Each connection gets its own thread; every line goes to [`rpc::handle_line`].

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use crate::rpc;

/// Why `arch serve` cannot start.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// Another engine answers on this repo's socket.
    #[error("{0}: an engine is already serving this repo")]
    AlreadyServing(PathBuf),
    /// The socket directory belongs to another user or is open to others.
    #[error("{path}: {reason}; refusing to serve from it")]
    UnsafeDir {
        /// The directory.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// The repo root is not valid Unicode, so arch-app cannot hash it the same way.
    #[error("{0}: the repo path is not valid Unicode")]
    NotUnicode(PathBuf),
    /// A file or socket operation failed.
    #[error("{path}: {source}")]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> ServeError + '_ {
    move |source| ServeError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// What the socket path depends on besides the repo root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketEnv {
    /// `$XDG_RUNTIME_DIR`; unset or empty means none.
    pub runtime_dir: Option<OsString>,
    /// This user's id.
    pub uid: u32,
}

impl SocketEnv {
    /// This process's environment.
    pub fn current() -> Self {
        SocketEnv {
            runtime_dir: std::env::var_os("XDG_RUNTIME_DIR"),
            uid: rustix::process::getuid().as_raw(),
        }
    }
}

/// FNV-1a 32-bit over the UTF-16 code units of `s`, 8 lowercase hex digits: arch-app's `hash8`.
pub fn hash8(s: &str) -> String {
    let hash = s.encode_utf16().fold(0x811c_9dc5_u32, |h, unit| {
        (h ^ u32::from(unit)).wrapping_mul(0x0100_0193)
    });
    format!("{hash:08x}")
}

/// The socket for the repo at `root`, taken as given (callers resolve the real path first).
pub fn socket_path(root: &Path, env: &SocketEnv) -> Result<PathBuf, ServeError> {
    let text = root
        .to_str()
        .ok_or_else(|| ServeError::NotUnicode(root.to_path_buf()))?;
    let name = format!("{}.sock", hash8(text.trim_end_matches(['/', '\\'])));
    let runtime = env
        .runtime_dir
        .as_ref()
        .and_then(|d| d.to_str())
        .map(|d| d.trim_end_matches('/'))
        .filter(|d| !d.is_empty());
    Ok(match runtime {
        Some(dir) => Path::new(dir).join("arch").join(name),
        None => PathBuf::from(format!("/tmp/arch-{}", env.uid)).join(name),
    })
}

/// A bound socket, ready to [`run`](Server::run).
#[derive(Debug)]
pub struct Server {
    listener: UnixListener,
    path: PathBuf,
}

/// Bind the socket for the repo at `repo`: resolve its real path, make the directory private,
/// refuse when an engine already answers, replace a stale socket.
pub fn bind(repo: &Path, env: &SocketEnv) -> Result<Server, ServeError> {
    let root = repo.canonicalize().map_err(io(repo))?;
    let path = socket_path(&root, env)?;
    let dir = path.parent().expect("a socket path has a directory");
    private_dir(dir, env.uid)?;
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            return Err(ServeError::AlreadyServing(path));
        }
        std::fs::remove_file(&path).map_err(io(&path))?;
    }
    let listener = UnixListener::bind(&path).map_err(io(&path))?;
    Ok(Server { listener, path })
}

/// Create `dir` as `0700`, or check that the existing one is this user's and closed to others.
fn private_dir(dir: &Path, uid: u32) -> Result<(), ServeError> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(io(dir))?;
    let meta = std::fs::symlink_metadata(dir).map_err(io(dir))?;
    let unsafe_dir = |reason: String| ServeError::UnsafeDir {
        path: dir.to_path_buf(),
        reason,
    };
    if !meta.is_dir() {
        return Err(unsafe_dir("not a directory".into()));
    }
    if meta.uid() != uid {
        return Err(unsafe_dir(format!(
            "owned by uid {}, not {uid}",
            meta.uid()
        )));
    }
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(unsafe_dir(format!("mode {mode:o} lets other users in")));
    }
    Ok(())
}

impl Server {
    /// Where the socket is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Accept connections until the process ends, one thread each.
    pub fn run(self) -> Result<(), ServeError> {
        for stream in self.listener.incoming() {
            let stream = stream.map_err(io(&self.path))?;
            std::thread::spawn(move || connection(stream));
        }
        Ok(())
    }
}

/// One client: a reply line per request line, until it hangs up.
fn connection(stream: UnixStream) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        if let Some(reply) = rpc::handle_line(&line)
            && writeln!(writer, "{reply}")
                .and_then(|()| writer.flush())
                .is_err()
        {
            return;
        }
    }
}
