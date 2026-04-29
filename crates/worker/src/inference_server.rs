//! Worker-local llama.cpp `llama-server` for single-node inference.
//!
//! When `MODEL_PATH` points at a readable GGUF file, the worker spawns
//! `llama-server` (built from our llama.cpp submodule) on `INFERENCE_PORT`.
//! That gives every worker a self-contained OpenAI-compatible HTTP endpoint
//! the cluster's `openai-api` can forward chat requests to.
//!
//! Multi-node tensor-parallel inference (using `rpc-server` from the same
//! llama.cpp build) is the next step — it reuses the same model file but
//! flips the launch command to `llama-cli --rpc <peer1>:50052,<peer2>:50052`.
//! For now: one worker, one model, one HTTP endpoint.
//!
//! Lifecycle: the spawned process is owned by the returned `InferenceServer`.
//! Dropping it kills the child — important so the agent reliably tears down
//! the inference process on shutdown / restart.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// Default port for the worker-local llama-server. Distinct from the
/// `rpc-server-ext` port (50052) so both can coexist on the same host.
pub const DEFAULT_INFERENCE_PORT: u16 = 50053;

/// Possible binary names — `llama-server` is the upstream target. We also
/// look for `llama-cpp-server` as a fall-through for distros that rename it.
const BIN_NAMES: &[&str] = &["llama-server", "llama-cpp-server"];

/// Search paths beyond `$PATH`. Order matters: dev workflow first, then
/// container / package layouts.
const SEARCH_DIRS: &[&str] = &[
    "/usr/local/bin",
    "/opt/gpucluster/bin",
    "/opt/homebrew/bin",
];

pub struct InferenceServer {
    pub child: Option<Child>,
    pub port: u16,
    pub model_path: String,
    pub bind_host: String,
}

impl InferenceServer {
    /// Spawn `llama-server` if the environment is set up for it; otherwise
    /// returns `Ok(None)` so the worker can run inference-ineligible (it still
    /// shows up in the dashboard).
    pub fn try_spawn(port: u16) -> Result<Option<Self>> {
        let model_path = match std::env::var("MODEL_PATH") {
            Ok(p) if !p.is_empty() => p,
            _ => {
                tracing::info!("MODEL_PATH not set — skipping inference server");
                return Ok(None);
            }
        };
        if !Path::new(&model_path).is_file() {
            tracing::warn!(%model_path, "MODEL_PATH does not point at a file — skipping");
            return Ok(None);
        }
        let bin = locate_binary().ok_or_else(|| {
            anyhow!(
                "llama-server binary not found on $PATH or in the standard \
                 install dirs. Build cpp/llama-rpc-ext (it builds llama-server \
                 alongside rpc-server) and symlink it into ~/.local/bin."
            )
        })?;

        // Bind on all interfaces so the openai-api dispatcher can reach it
        // over the WireGuard mesh (or LAN in dev). The mTLS layer in the
        // gateway is what makes the public-facing surface safe; this socket
        // sits on the worker's private cluster IP only.
        let bind_host = std::env::var("INFERENCE_BIND")
            .unwrap_or_else(|_| "0.0.0.0".to_string());

        // Phase 2 default: keep it simple — full GPU offload, fixed ctx,
        // no quantisation override. Tunables come once the scheduler can
        // inspect a job spec.
        let mut cmd = Command::new(&bin);
        cmd.arg("--host").arg(&bind_host)
           .arg("--port").arg(port.to_string())
           .arg("--model").arg(&model_path)
           .arg("--n-gpu-layers").arg("999")
           .arg("--ctx-size").arg(
               std::env::var("INFERENCE_CTX").unwrap_or_else(|_| "4096".into())
           );

        // Optional jinja chat-template support — many GGUF models embed one,
        // so we let llama-server pick it up automatically.
        cmd.arg("--jinja");

        let child = cmd
            .spawn()
            .with_context(|| format!("spawn {}", bin.display()))?;

        tracing::info!(
            port,
            pid = child.id(),
            bin = %bin.display(),
            %model_path,
            "llama-server started"
        );
        Ok(Some(Self {
            child: Some(child),
            port,
            model_path,
            bind_host,
        }))
    }

    /// HTTP base URL for downstream services (openai-api dispatcher) to
    /// forward requests to. Intentionally bound to 0.0.0.0 here — the
    /// coordinator tells the dispatcher which IP to actually use.
    pub fn local_health_url(&self) -> String {
        format!("http://127.0.0.1:{}/health", self.port)
    }
}

impl Drop for InferenceServer {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn locate_binary() -> Option<PathBuf> {
    if let Ok(path_env) = std::env::var("PATH") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        for dir in path_env.split(sep) {
            for name in BIN_NAMES {
                let p = PathBuf::from(dir).join(name);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    for dir in SEARCH_DIRS {
        for name in BIN_NAMES {
            let p = PathBuf::from(dir).join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}
