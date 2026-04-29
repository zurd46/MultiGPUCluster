//! Worker-local llama.cpp `llama-server` for single-node inference.
//!
//! When the worker has a GGUF model on disk it spawns `llama-server` (built
//! from our llama.cpp submodule) on `INFERENCE_PORT`. That gives every worker
//! a self-contained OpenAI-compatible HTTP endpoint the cluster's `openai-api`
//! can forward chat requests to.
//!
//! The model can come from two places:
//!
//!   1. The `MODEL_PATH` env var, set at worker startup. Used for static
//!      deployments where the GGUF is provisioned by ops (e.g. baked into a
//!      container image).
//!   2. A runtime `load_model` control RPC from the coordinator (admin UI →
//!      mgmt-backend → coordinator → worker). The worker downloads the GGUF
//!      from Hugging Face, stops the running llama-server, and respawns it
//!      against the new file.
//!
//! Lifecycle: the spawned process is owned by `Supervisor`. Calling
//! `Supervisor::switch_to(...)` cleanly tears the previous child down and
//! starts a new one. Dropping the supervisor kills the child — important so
//! the agent reliably tears down the inference process on shutdown.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use tokio::sync::Mutex;

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

/// State shared between the heartbeat loop and the control server. Wrapped
/// in `Arc<Mutex<…>>` so a `load_model` RPC can mutate it from the control
/// task while the heartbeat task reads `current_model` for reporting.
pub type SharedSupervisor = Arc<Mutex<Supervisor>>;

pub struct Supervisor {
    port: u16,
    bind_host: String,
    /// Currently running llama-server child, if any. `None` between the
    /// "killed old" and "spawned new" steps of a model switch, and during
    /// normal worker startup before any model is loaded.
    child: Option<Child>,
    /// Path of the model llama-server is currently serving. Empty / None
    /// when nothing is loaded.
    pub model_path: Option<String>,
    /// Logical id of the currently loaded model. This is what the worker
    /// puts in its heartbeat — the path is an implementation detail that
    /// the cluster doesn't care about.
    pub model_id: Option<String>,
}

impl Supervisor {
    /// Build a supervisor bound to `port`, optionally pre-launching
    /// llama-server from `MODEL_PATH` so static deployments still work.
    /// Failures to spawn are non-fatal — they leave the supervisor in the
    /// "no model loaded" state, ready to receive a `load_model` RPC later.
    pub fn boot(port: u16) -> Self {
        let bind_host =
            std::env::var("INFERENCE_BIND").unwrap_or_else(|_| "0.0.0.0".to_string());
        let mut sup = Self {
            port,
            bind_host,
            child: None,
            model_path: None,
            model_id: None,
        };
        if let Ok(p) = std::env::var("MODEL_PATH") {
            if !p.is_empty() && Path::new(&p).is_file() {
                // Inferred id from filename — the static-deployment path
                // never reports a "real" model id, just whatever the file
                // is called. The runtime load_model path always sets a
                // proper id.
                let inferred_id = Path::new(&p)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("local")
                    .to_string();
                if let Err(e) = sup.spawn_llama_server(&p) {
                    tracing::warn!(error = %e, "llama-server failed to start from MODEL_PATH; inference disabled until load_model is called");
                } else {
                    sup.model_path = Some(p);
                    sup.model_id = Some(inferred_id);
                }
            } else if !p.is_empty() {
                tracing::warn!(%p, "MODEL_PATH does not point at a file — skipping initial spawn");
            }
        } else {
            tracing::info!("MODEL_PATH not set — waiting for load_model RPC before serving inference");
        }
        sup
    }

    /// Stop the current llama-server (if any) and start a new one against
    /// `path`. On success, `model_id` and `model_path` reflect the new
    /// state; on failure the supervisor is left with no running child and
    /// the previous model_id/path cleared (so heartbeats stop advertising
    /// inference until a retry succeeds).
    pub fn switch_to(&mut self, model_id: String, path: String) -> Result<()> {
        // Tear down the old child first so the new one can claim the port.
        // We don't wait long — the kernel will reap.
        if let Some(mut old) = self.child.take() {
            let _ = old.kill();
            let _ = old.wait();
            tracing::info!("previous llama-server stopped");
        }
        self.model_id = None;
        self.model_path = None;

        match self.spawn_llama_server(&path) {
            Ok(()) => {
                self.model_path = Some(path);
                self.model_id = Some(model_id);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Spawn `llama-server` on the supervisor's port, pointing at `path`.
    /// On success, stores the child in `self.child`.
    fn spawn_llama_server(&mut self, path: &str) -> Result<()> {
        if !Path::new(path).is_file() {
            return Err(anyhow!("model path does not exist: {path}"));
        }
        let bin = locate_binary().ok_or_else(|| {
            anyhow!(
                "llama-server binary not found on $PATH or in the standard \
                 install dirs. Build cpp/llama-rpc-ext (it builds llama-server \
                 alongside rpc-server) and symlink it into ~/.local/bin."
            )
        })?;
        let mut cmd = Command::new(&bin);
        cmd.arg("--host").arg(&self.bind_host)
           .arg("--port").arg(self.port.to_string())
           .arg("--model").arg(path)
           .arg("--n-gpu-layers").arg("999")
           .arg("--ctx-size").arg(
               std::env::var("INFERENCE_CTX").unwrap_or_else(|_| "4096".into())
           )
           .arg("--jinja");

        let child = cmd
            .spawn()
            .with_context(|| format!("spawn {}", bin.display()))?;

        tracing::info!(
            port = self.port,
            pid = child.id(),
            bin = %bin.display(),
            model_path = %path,
            "llama-server started"
        );
        self.child = Some(child);
        Ok(())
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Worker advertises just `:<port>` on the heartbeat — the coordinator
    /// pairs it with the observed public IP. Returns `None` when no model
    /// is currently loaded so the dispatcher correctly skips us.
    pub fn endpoint_advertise(&self) -> Option<String> {
        self.child.as_ref().map(|_| format!(":{}", self.port))
    }
}

impl Drop for Supervisor {
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
