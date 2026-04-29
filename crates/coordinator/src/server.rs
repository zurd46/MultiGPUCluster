use anyhow::Result;
use axum::{
    extract::{ConnectInfo, Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use gpucluster_proto::node as pb;
use gpucluster_sysinfo::inventory;
use serde_json::{json, Value};
use std::net::SocketAddr;
use tonic::transport::Server;

use crate::{
    config::CoordConfig,
    heartbeat,
    registry::Registry,
    service::CoordSvc,
};
use gpucluster_proto::coordinator::coordinator_service_server::CoordinatorServiceServer;

pub async fn run(cfg: CoordConfig) -> Result<()> {
    let registry = Registry::new();

    tokio::spawn(heartbeat::watchdog(registry.clone()));

    let http = run_http(&cfg.http_bind, registry.clone());
    let grpc = run_grpc(&cfg.grpc_bind, registry.clone());

    tokio::try_join!(http, grpc)?;
    Ok(())
}

async fn run_http(bind: &str, reg: Registry) -> Result<()> {
    let app = Router::new()
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/nodes", get(list_nodes))
        // POST /nodes/report — workers upload their full inventory snapshot
        // here (proxied via /cluster/nodes/report on the gateway). Acts as
        // upsert-by-node_id, so it doubles as registration AND heartbeat.
        .route("/nodes/report", post(report_node))
        // GET /nodes/eligible — dispatch-time view: only nodes that can
        // currently serve an inference request (status=online, GPU present,
        // recent heartbeat). Phase 2 scheduler reads this.
        .route("/nodes/eligible", get(eligible_nodes))
        // POST /nodes/{id}/load_model — control-plane proxy. mgmt-backend
        // calls this with `{model_id, hf_repo, hf_file, hf_token, …}`; we
        // look up the worker's control_endpoint from the registry and
        // forward the JSON unchanged.
        .route("/nodes/{id}/load_model", post(load_model_proxy))
        .with_state(reg);

    let addr: SocketAddr = bind.parse()?;
    tracing::info!(%addr, "coordinator http listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await?;
    Ok(())
}

async fn list_nodes(State(reg): State<Registry>) -> Json<Value> {
    // Full inventory per node — dashboard + CLI consume this. Same JSON shape
    // as what the worker uploads, so a round-trip is loss-less.
    let nodes: Vec<Value> = reg.list().into_iter().map(|e| {
        let mut obj = inventory::to_json(&e.info);
        // Decorate with coordinator-side state that isn't part of the worker
        // snapshot (last seen time, current_public_ip captured at TLS socket).
        if let Value::Object(map) = &mut obj {
            map.insert("last_heartbeat".into(), Value::String(e.last_heartbeat.to_rfc3339()));
            map.insert("current_public_ip".into(),
                e.current_public_ip.clone().map(Value::String).unwrap_or(Value::Null));
            // Status as the human-readable lowercase label the dashboard expects
            // ("online", "pending_approval", …). Raw enum number stays available
            // under `status_code` for clients that want to switch on it.
            map.insert("status".into(), Value::String(status_label(e.info.status).into()));
            map.insert("status_code".into(), Value::Number(e.info.status.into()));
            // Dashboard table reads `id`; keep `node_id` for the canonical proto
            // name. One row, two keys — costs nothing, avoids touching admin_ui.
            map.insert("id".into(), Value::String(e.info.node_id.clone()));
            // Advertise the loaded model + control endpoint so the admin UI
            // can show a "Load model" button only for nodes that can accept
            // one (those with a control endpoint), and pre-disable it for
            // nodes already running the requested model.
            map.insert("current_model".into(),
                e.current_model.clone().map(Value::String).unwrap_or(Value::Null));
            map.insert("control_endpoint".into(),
                e.control_endpoint.clone().map(Value::String).unwrap_or(Value::Null));
        }
        obj
    }).collect();
    Json(json!({ "count": nodes.len(), "nodes": nodes }))
}

fn status_label(status: i32) -> &'static str {
    match pb::NodeStatus::try_from(status).unwrap_or(pb::NodeStatus::Unspecified) {
        pb::NodeStatus::Unspecified      => "unspecified",
        pb::NodeStatus::PendingApproval  => "pending_approval",
        pb::NodeStatus::Online           => "online",
        pb::NodeStatus::Degraded         => "degraded",
        pb::NodeStatus::Draining         => "draining",
        pb::NodeStatus::Offline          => "offline",
        pb::NodeStatus::Quarantined      => "quarantined",
        pb::NodeStatus::Revoked          => "revoked",
    }
}

/// Phase 2 dispatch helper: returns a slim view of nodes that can take a job
/// right now. Filters out anything not actively heartbeating with a usable GPU.
async fn eligible_nodes(State(reg): State<Registry>) -> Json<Value> {
    use chrono::{Duration as ChronoDuration, Utc};
    let stale = ChronoDuration::seconds(60);
    let now = Utc::now();

    let eligible: Vec<Value> = reg
        .list()
        .into_iter()
        .filter(|e| {
            // Heartbeat fresh AND has at least one GPU AND not in a terminal
            // state. We're permissive on `pending_approval` for dev setups —
            // production would gate that behind admin approval.
            now.signed_duration_since(e.last_heartbeat) < stale
                && !e.info.gpus.is_empty()
                && !matches!(
                    pb::NodeStatus::try_from(e.info.status).unwrap_or(pb::NodeStatus::Unspecified),
                    pb::NodeStatus::Revoked
                        | pb::NodeStatus::Quarantined
                        | pb::NodeStatus::Draining
                        | pb::NodeStatus::Offline
                )
        })
        .map(|e| {
            let primary = e.info.gpus.first();
            // Prefer the WireGuard IP once the mesh is up; fall back to the
            // socket-observed public IP for dev / pre-mesh.
            let wg_ip = e
                .info
                .network
                .as_ref()
                .map(|n| n.wg_ip.clone())
                .filter(|s| !s.is_empty());
            let dispatch_ip = wg_ip.clone().or(e.current_public_ip.clone());
            // Build a fully-qualified URL for the openai-api dispatcher.
            // `inference_endpoint` is either `":port"` (port-only — pair with
            // the observed dispatch IP) or `"host:port"` (already complete,
            // typical for `INFERENCE_ADVERTISED_HOST=host.docker.internal`).
            let inference_url = match &e.inference_endpoint {
                Some(ep) if ep.starts_with(':') => {
                    dispatch_ip.as_ref().map(|ip| format!("http://{ip}{ep}"))
                }
                Some(ep) => Some(format!("http://{ep}")),
                None => None,
            };
            json!({
                "node_id":         e.info.node_id,
                "device_name":     e.info.os.as_ref().map(|o| o.device_name.clone()),
                "wg_ip":           e.info.network.as_ref().map(|n| n.wg_ip.clone()),
                "public_ip":       e.current_public_ip,
                "rpc_port":        gpucluster_common::ports::WORKER_RPC,
                "inference_url":   inference_url,
                "gpu": primary.map(|g| json!({
                    "name":         g.name,
                    "backend":      pb::GpuBackend::try_from(g.backend)
                        .ok().map(|b| match b {
                            pb::GpuBackend::Cuda => "cuda",
                            pb::GpuBackend::Metal => "metal",
                            pb::GpuBackend::Rocm => "rocm",
                            pb::GpuBackend::Vulkan => "vulkan",
                            _ => "unspecified",
                        }),
                    "architecture": g.architecture,
                    "vram_total":   g.vram_total_bytes,
                    "vram_free":    g.vram_free_bytes,
                    "core_count":   g.gpu_core_count,
                })),
            })
        })
        .collect();

    Json(json!({ "count": eligible.len(), "nodes": eligible }))
}

async fn report_node(
    State(reg): State<Registry>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Single-shot serde decode replaces a 110-line hand-rolled parser. The
    // sidecar fields (inference_endpoint / control_endpoint / current_model)
    // come along for free because they're declared on `NodeInfoView`.
    let view: gpucluster_common::nodes::NodeInfoView =
        match serde_json::from_value(body.clone()) {
            Ok(v) => v,
            Err(e) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "invalid_payload", "message": e.to_string() })),
                ));
            }
        };
    let inference_endpoint = view.inference_endpoint.clone().filter(|s| !s.is_empty());
    let control_endpoint = view.control_endpoint.clone().filter(|s| !s.is_empty());
    let current_model = view.current_model.clone().filter(|s| !s.is_empty());
    let info: pb::NodeInfo = view.into();
    if info.node_id.is_empty() {
        return Err((StatusCode::BAD_REQUEST,
            Json(json!({ "error": "missing_node_id" }))));
    }

    // Duplicate-worker guard: if another worker with the same hardware
    // fingerprint is still actively heartbeating under a different node_id,
    // reject this report. The second process is almost certainly an
    // accidental dual-start (different --data-dir → different node.id file,
    // same physical machine). We use 60 s as the freshness window — same
    // value the watchdog uses to mark nodes offline — so once the original
    // worker truly goes away, the new one can take over without operator
    // intervention.
    if let Some(existing) = reg.find_active_with_hw_fingerprint(
        &info.hw_fingerprint,
        &info.node_id,
        chrono::Duration::seconds(60),
    ) {
        tracing::warn!(
            new_node_id = %info.node_id,
            existing_node_id = %existing,
            hw_fingerprint = %info.hw_fingerprint,
            "rejecting duplicate worker registration",
        );
        return Err((
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_hw_fingerprint",
                "message": format!(
                    "another worker on this hardware is already active as node {existing}. \
                     Stop the other worker (or wait ~60 s for its heartbeat to expire) \
                     before re-registering with a new node_id."
                ),
                "active_node_id": existing,
                "hw_fingerprint": info.hw_fingerprint,
            })),
        ));
    }

    let public_ip = Some(addr.ip().to_string());
    let id = info.node_id.clone();
    let device = info.os.as_ref().map(|o| o.device_name.clone()).unwrap_or_default();
    let gpu_count = info.gpus.len();
    let has_inference = inference_endpoint.is_some();
    reg.upsert(info, public_ip, inference_endpoint, control_endpoint, current_model);
    tracing::info!(%id, %device, gpus = gpu_count, inference = has_inference, "node inventory updated");
    Ok(Json(json!({ "ok": true, "node_id": id })))
}

/// POST /nodes/{id}/load_model — proxy from mgmt-backend to the worker's
/// control endpoint. The body is forwarded verbatim. We deliberately don't
/// unmarshal/remarshal it: the worker owns the schema, and a pass-through
/// keeps the coordinator out of the upgrade dance every time we add a field.
///
/// Returns:
///   * 502 if the worker is unknown, never advertised a control endpoint,
///     or doesn't have a public IP yet (heartbeat hasn't landed).
///   * Whatever status the worker returns otherwise.
async fn load_model_proxy(
    State(reg): State<Registry>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let entry = reg.get(&id).ok_or_else(|| {
        (StatusCode::BAD_GATEWAY,
            Json(json!({ "error": "unknown_node", "node_id": id.clone() })))
    })?;

    // control_endpoint can be either `":port"` (port-only — pair with the
    // observed dispatch IP) or `"host:port"` (already complete, typical for
    // `CONTROL_ADVERTISED_HOST=host.docker.internal`). Same dance as
    // inference_url in eligible_nodes.
    let port_part = entry.control_endpoint.clone().ok_or_else(|| {
        (StatusCode::BAD_GATEWAY,
            Json(json!({
                "error":   "node_has_no_control_endpoint",
                "node_id": id.clone(),
                "hint":    "worker may be running an older version that doesn't expose /control",
            })))
    })?;
    let url = if port_part.starts_with(':') {
        let wg_ip = entry
            .info
            .network
            .as_ref()
            .map(|n| n.wg_ip.clone())
            .filter(|s| !s.is_empty());
        let host = wg_ip.or(entry.current_public_ip.clone()).ok_or_else(|| {
            (StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "node_has_no_routable_ip", "node_id": id.clone() })))
        })?;
        format!("http://{host}{port_part}/control/load_model")
    } else {
        format!("http://{port_part}/control/load_model")
    };

    // The URL we built ends with `/control/load_model`, but
    // `WorkerControlClient::load_model` appends that suffix itself. Strip
    // it so the client builds the same final URL without us double-encoding
    // the path.
    let base_url = url
        .strip_suffix("/control/load_model")
        .unwrap_or(&url)
        .to_string();
    let client = gpucluster_common::clients::WorkerControlClient::new(base_url);
    match client.load_model(&body).await {
        Ok(json) => {
            tracing::info!(%id, %url, "load_model dispatched to worker");
            Ok(Json(json))
        }
        Err(gpucluster_common::clients::ClientError::Upstream { status, body, .. }) => {
            // Forward the worker's own response body (parsed if it's valid
            // JSON, otherwise raw text wrapped). Lets the admin UI surface
            // whatever the worker said about why it rejected.
            let parsed = serde_json::from_str::<Value>(&body).unwrap_or_else(|_| json!({ "raw": body }));
            Err((
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error":         "worker_rejected",
                    "worker_status": status,
                    "worker_body":   parsed,
                })),
            ))
        }
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error":   "worker_unreachable",
                "url":     url,
                "message": e.to_string(),
            })),
        )),
    }
}

async fn run_grpc(bind: &str, reg: Registry) -> Result<()> {
    let svc = CoordSvc { registry: reg };
    let addr: SocketAddr = bind.parse()?;
    tracing::info!(%addr, "coordinator grpc listening");
    Server::builder()
        .add_service(CoordinatorServiceServer::new(svc))
        .serve(addr)
        .await?;
    Ok(())
}
