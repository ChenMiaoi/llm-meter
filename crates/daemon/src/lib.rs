pub mod alerts;
pub mod config;
pub mod ipc;
pub mod local_codex;
pub mod notifications;
mod runner;
pub mod scheduler;
pub mod snapshot;
pub mod telemetry;

pub use runner::run;

use std::{io, path::PathBuf};

/// Persistent application root. Runtime sockets deliberately stay under
/// XDG_RUNTIME_DIR, and secrets stay in the operating-system keyring.
pub fn app_home() -> Result<PathBuf, io::Error> {
    std::env::var_os("LLM_METER_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|value| PathBuf::from(value).join(".llm-meter")))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is required"))
}

pub fn runtime_dir() -> Result<PathBuf, io::Error> {
    let base = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "XDG_RUNTIME_DIR is required; refusing an unsafe /tmp fallback",
        )
    })?;
    Ok(PathBuf::from(base).join("llm-meter"))
}
pub fn socket_path() -> Result<PathBuf, io::Error> {
    Ok(runtime_dir()?.join("daemon.sock"))
}
