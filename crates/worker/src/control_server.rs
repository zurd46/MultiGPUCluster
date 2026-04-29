//! Worker-local HTTP control plane.
//!
//! The coordinator forwards admin-initiated commands here (today: just
//! `load_model`). Listens on a separate port from the inference server so
//! they can be lifecycled independently — the control server stays up even
//! when llama-server is being restarted to swap models.
//!
//! Trust model: same as `inference_server`. The endpoint binds to whatever
//! `CONTROL_BIND` says (default `0.0.0.0`) and relies on the cluster network
//! perimeter (WireGuard mesh in production, LAN in dev) for confidentiality.
//! When mTLS is added cluster-wide, this server gets the same client-cert
//! check that the coordinator already has.

use crate::inference_server::SharedSupervisor;
use crate::model_downloader::{self, DownloadRequest};
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Re-export of [`gpucluster_common::ports::WORKER_CONTROL`] so existing
/// `agent.rs` callers don't need to chase imports.
pub const DEFAULT_CONTROL_PORT: u16 = gpucluster_common::ports::WORKER_CONTROL;

#[derive(Clone)]
struct ControlState {
    sup: SharedSupervisor,
    /// `data-dir/models/` — where downloaded GGUFs land.
    models_dir: Arc<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct LoadModelReq {
    model_id: String,
    hf_repo: String,
    hf_file: String,
    /// Optional — empty string means "no auth header on the HF download".
    #[serde(default)]
    hf_token: String,
    /// Filename to save as on disk (under data-dir/models/). Defaults to
    /// `hf_file` when empty.
    #[serde(default)]
    local_filename: String,
}

#[derive(Debug, Serialize)]
struct LoadModelAck {
    ok: bool,
    accepted: bool,
    model_id: String,
    /// "downloading" — the actual switch happens in a background task. Poll
    /// the worker's heartbeat (`current_model`) to learn when it completes.
    state: &'static str,
}

/// Spawn the control server on `port` and return its task handle. Failure
/// to bind is non-fatal: the worker just runs without a control plane (it
/// can still report inventory + serve inference; remote model-loading is
/// the only feature that needs this listener).
pub fn spawn(port: u16, sup: SharedSupervisor, data_dir: &str) -> Option<JoinHandle<()>> {
    let bind_host = std::env::var("CONTROL_BIND").unwrap_or_else(|_| "0.0.0.0".to_string());
    let addr: SocketAddr = match format!("{bind_host}:{port}").parse() {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, "control server bind addr invalid; control plane disabled");
            return None;
        }
    };
    let models_dir = Arc::new(PathBuf::from(data_dir).join("models"));
    let state = ControlState { sup, models_dir };

    let app = Router::new()
        .route("/control/load_model", post(load_model))
        .with_state(state);

    let handle = tokio::spawn(async move {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                tracing::info!(%addr, "control server listening");
                if let Err(e) = axum::serve(listener, app).await {
                    tracing::warn!(error = %e, "control server exited");
                }
            }
            Err(e) => tracing::warn!(error = %e, %addr, "control server failed to bind"),
        }
    });
    Some(handle)
}

/// Endpoint for the heartbeat to advertise. By default the IP is filled in
/// by the coordinator from the socket address. When `CONTROL_ADVERTISED_HOST`
/// (or `INFERENCE_ADVERTISED_HOST` as a fallback — they always point at the
/// same machine) is set, the worker advertises `<host>:<port>` directly so
/// the coordinator passes it through unchanged. This is the escape hatch for
/// Mac dev: Docker bridge gateway → use `host.docker.internal`.
pub fn endpoint_advertise(port: u16) -> String {
    let host_override = std::env::var("CONTROL_ADVERTISED_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("INFERENCE_ADVERTISED_HOST")
                .ok()
                .filter(|s| !s.is_empty())
        });
    match host_override {
        Some(h) => format!("{h}:{port}"),
        None => format!(":{port}"),
    }
}

async fn load_model(
    State(s): State<ControlState>,
    Json(req): Json<LoadModelReq>,
) -> Result<Json<LoadModelAck>, (StatusCode, Json<Value>)> {
    if req.model_id.is_empty() || req.hf_repo.is_empty() || req.hf_file.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "model_id, hf_repo, hf_file are all required" })),
        ));
    }
    let local_filename = if req.local_filename.is_empty() {
        req.hf_file.clone()
    } else {
        req.local_filename.clone()
    };
    let model_id = req.model_id.clone();
    let models_dir = s.models_dir.clone();
    let sup = s.sup.clone();

    // Detached task. The HTTP response returns immediately — admins watch
    // the heartbeat-reported `current_model` to learn when the swap is done.
    tokio::spawn(async move {
        let dl = DownloadRequest {
            repo: &req.hf_repo,
            file: &req.hf_file,
            local_filename: &local_filename,
            token: &req.hf_token,
        };
        let path = match model_downloader::fetch(dl, models_dir.as_path()).await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(error = %e, model_id = %req.model_id, "model download failed");
                return;
            }
        };
        let path_str = match path.to_str() {
            Some(s) => s.to_string(),
            None => {
                tracing::error!(path = ?path, "downloaded path is not valid utf-8");
                return;
            }
        };
        let mut guard = sup.lock().await;
        if let Err(e) = guard.switch_to(req.model_id.clone(), path_str) {
            tracing::error!(error = %e, model_id = %req.model_id, "llama-server switch failed");
        }
    });

    Ok(Json(LoadModelAck {
        ok: true,
        accepted: true,
        model_id,
        state: "downloading",
    }))
}
