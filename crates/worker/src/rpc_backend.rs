//! Worker-local launcher for the inference RPC server.
//!
//! The worker doesn't know in advance whether it sits on a CUDA box or an
//! Apple Silicon machine — it inspects its own GPU inventory (already
//! collected by `gpucluster-sysinfo`) and launches the matching variant of
//! `rpc-server-ext`. We keep this in the worker (not the bootstrapper) so
//! the worker can restart its own RPC server when assigned to new jobs
//! without going through the agent.

use anyhow::{anyhow, Context, Result};
use gpucluster_proto::node as pb;
use std::path::PathBuf;
use std::process::{Child, Command};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RpcBackend {
    Cuda,
    Metal,
    /// No GPU available — the worker still enrolls (so it shows up in the
    /// dashboard), but won't be eligible for inference jobs.
    None,
}

impl RpcBackend {
    pub fn from_inventory(gpus: &[pb::GpuInfo]) -> Self {
        // Pick the first non-unspecified backend — we don't yet support a
        // mixed CUDA+Metal box (would require two RPC servers, and is
        // physically impossible anyway: macOS hosts have no CUDA).
        for g in gpus {
            match pb::GpuBackend::try_from(g.backend).unwrap_or(pb::GpuBackend::Unspecified) {
                pb::GpuBackend::Cuda  => return RpcBackend::Cuda,
                pb::GpuBackend::Metal => return RpcBackend::Metal,
                _ => continue,
            }
        }
        RpcBackend::None
    }

    pub fn label(&self) -> &'static str {
        match self {
            RpcBackend::Cuda  => "cuda",
            RpcBackend::Metal => "metal",
            RpcBackend::None  => "none",
        }
    }
}

/// Search paths for the RPC binary. Order matters — dev workflow first
/// (target/), then container layout, then macOS package layout.
const RPC_BIN_NAME: &str = "rpc-server-ext";
const SEARCH_DIRS: &[&str] = &[
    "/usr/local/bin",
    "/opt/gpucluster/bin",
    "/opt/homebrew/bin",
];

pub struct RpcServer {
    pub backend: RpcBackend,
    pub child: Option<Child>,
    pub listen_port: u16,
}

impl RpcServer {
    pub fn spawn(backend: RpcBackend, listen_port: u16) -> Result<Self> {
        if backend == RpcBackend::None {
            tracing::info!("no GPU available — skipping RPC server start");
            return Ok(Self { backend, child: None, listen_port });
        }
        let bin = locate_binary().ok_or_else(|| {
            anyhow!(
                "rpc-server-ext binary not found. Build cpp/llama-rpc-ext with \
                 -DBUILD_RPC_SERVER=ON and ship it alongside the worker."
            )
        })?;

        let child = Command::new(&bin)
            .args(["--host", "0.0.0.0"])
            .args(["--port", &listen_port.to_string()])
            .spawn()
            .with_context(|| format!("spawn {}", bin.display()))?;

        tracing::info!(
            backend = backend.label(),
            port = listen_port,
            pid = child.id(),
            bin = %bin.display(),
            "rpc-server-ext started",
        );
        Ok(Self { backend, child: Some(child), listen_port })
    }
}

impl Drop for RpcServer {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn locate_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(if cfg!(windows) { ';' } else { ':' }) {
            let p = PathBuf::from(dir).join(RPC_BIN_NAME);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    for dir in SEARCH_DIRS {
        let p = PathBuf::from(dir).join(RPC_BIN_NAME);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}
