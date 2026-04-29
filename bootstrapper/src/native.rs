//! Native (no-Docker) worker spawn path.
//!
//! Used on macOS today, where Metal devices cannot be passed into a Linux
//! container and the worker therefore has to run as a regular launchd-managed
//! process alongside the agent. The .pkg installer drops both binaries into
//! /usr/local/bin so we just exec the sibling.

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::Command;

const WORKER_BIN_NAMES: &[&str] = &["gpucluster-worker"];
const SEARCH_DIRS: &[&str] = &["/usr/local/bin", "/opt/homebrew/bin", "/opt/gpucluster/bin"];

pub fn worker_binary_path() -> Option<PathBuf> {
    // 1. PATH lookup — covers `cargo install` / dev workflow.
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            for bin in WORKER_BIN_NAMES {
                let p = PathBuf::from(dir).join(bin);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    // 2. Fixed install locations — covers .pkg / Homebrew tap.
    for dir in SEARCH_DIRS {
        for bin in WORKER_BIN_NAMES {
            let p = PathBuf::from(dir).join(bin);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

pub fn worker_binary_present() -> bool {
    worker_binary_path().is_some()
}

pub fn run_worker(data_dir: &str, env: &[(&str, &str)]) -> Result<()> {
    let bin = worker_binary_path()
        .ok_or_else(|| anyhow!("gpucluster-worker not found on PATH or in /usr/local/bin"))?;

    let mut cmd = Command::new(&bin);
    cmd.env("NODE_DATA_DIR", data_dir);
    for (k, v) in env {
        cmd.env(k, v);
    }

    // Detach so launchd / the agent loop owns the worker as a sibling, not as
    // a child that disappears on agent restart. PID is recorded so `uninstall`
    // can clean it up.
    let child = cmd.spawn().with_context(|| format!("spawn {}", bin.display()))?;
    let pid = child.id();
    let _ = std::fs::write(pid_file(), pid.to_string());
    tracing::info!(%pid, bin = %bin.display(), "native worker spawned");
    Ok(())
}

pub fn stop_worker() -> Result<()> {
    let pid_path = pid_file();
    let pid_raw = match std::fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };
    if let Ok(pid) = pid_raw.trim().parse::<i32>() {
        // SIGTERM via /bin/kill — keeps us free of nix/libc deps in the agent.
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
    let _ = std::fs::remove_file(&pid_path);
    Ok(())
}

fn pid_file() -> PathBuf {
    crate::state::data_dir().join("worker.pid")
}
