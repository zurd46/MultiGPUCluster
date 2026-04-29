use anyhow::Result;
use axum::{
    extract::{ConnectInfo, State},
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
            // `inference_endpoint` from the worker is just `:port`; we glue.
            let inference_url = match (&dispatch_ip, &e.inference_endpoint) {
                (Some(ip), Some(port_part)) => Some(format!("http://{ip}{port_part}")),
                _ => None,
            };
            json!({
                "node_id":         e.info.node_id,
                "device_name":     e.info.os.as_ref().map(|o| o.device_name.clone()),
                "wg_ip":           e.info.network.as_ref().map(|n| n.wg_ip.clone()),
                "public_ip":       e.current_public_ip,
                "rpc_port":        50052,
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
    let info = match parse_node_info(&body) {
        Ok(i) => i,
        Err(e) => {
            return Err((StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid_payload", "message": e }))));
        }
    };
    if info.node_id.is_empty() {
        return Err((StatusCode::BAD_REQUEST,
            Json(json!({ "error": "missing_node_id" }))));
    }
    let public_ip = Some(addr.ip().to_string());
    let id = info.node_id.clone();
    let device = info.os.as_ref().map(|o| o.device_name.clone()).unwrap_or_default();
    let gpu_count = info.gpus.len();
    let inference_endpoint = body
        .get("inference_endpoint")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    let has_inference = inference_endpoint.is_some();
    reg.upsert(info, public_ip, inference_endpoint);
    tracing::info!(%id, %device, gpus = gpu_count, inference = has_inference, "node inventory updated");
    Ok(Json(json!({ "ok": true, "node_id": id })))
}

/// Hand-rolled inverse of `inventory::to_json`. We avoid pulling serde-derive
/// onto every prost message because that would force a build-time codegen
/// dependency for every consumer of the proto crate; doing it once here is
/// cheaper than the alternative.
fn parse_node_info(v: &Value) -> Result<pb::NodeInfo, String> {
    let obj = v.as_object().ok_or("body must be a JSON object")?;
    let s = |k: &str| obj.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let arr_str = |k: &str| obj.get(k).and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let os = obj.get("os").and_then(|v| v.as_object()).map(|o| pb::OsInfo {
        family:      o.get("family").and_then(|v| v.as_str()).unwrap_or_default().into(),
        version:     o.get("version").and_then(|v| v.as_str()).unwrap_or_default().into(),
        kernel:      o.get("kernel").and_then(|v| v.as_str()).unwrap_or_default().into(),
        arch:        o.get("arch").and_then(|v| v.as_str()).unwrap_or_default().into(),
        device_name: o.get("device_name").and_then(|v| v.as_str()).unwrap_or_default().into(),
    });

    let cpu_mem = obj.get("cpu_mem").and_then(|v| v.as_object()).map(|c| pb::CpuMemInfo {
        cpu_model:       c.get("cpu_model").and_then(|v| v.as_str()).unwrap_or_default().into(),
        cpu_cores:       c.get("cpu_cores").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        cpu_threads:     c.get("cpu_threads").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        ram_total_bytes: c.get("ram_total_bytes").and_then(|v| v.as_u64()).unwrap_or(0),
        ram_free_bytes:  c.get("ram_free_bytes").and_then(|v| v.as_u64()).unwrap_or(0),
    });

    let network = obj.get("network").and_then(|v| v.as_object()).map(|n| pb::NetworkInfo {
        public_ip_v4:        n.get("public_ip_v4").and_then(|v| v.as_str()).unwrap_or_default().into(),
        public_ip_v6:        n.get("public_ip_v6").and_then(|v| v.as_str()).unwrap_or_default().into(),
        asn:                 String::new(),
        isp:                 String::new(),
        country_code:        String::new(),
        city:                String::new(),
        public_ip_is_dynamic: false,
        public_ip_changed_at: 0,
        local_ips:           n.get("local_ips").and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        primary_iface:       n.get("primary_iface").and_then(|v| v.as_str()).unwrap_or_default().into(),
        link_speed_mbps:     n.get("link_speed_mbps").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        wg_ip:               n.get("wg_ip").and_then(|v| v.as_str()).unwrap_or_default().into(),
        wg_pubkey_sha:       n.get("wg_pubkey_sha").and_then(|v| v.as_str()).unwrap_or_default().into(),
        wg_listen_port:      n.get("wg_listen_port").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        rtt_to_gateway_ms:   n.get("rtt_to_gateway_ms").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
    });

    let gpus = obj.get("gpus").and_then(|v| v.as_array()).map(|arr| {
        arr.iter().filter_map(|g| g.as_object()).map(|g| {
            let backend_i = match g.get("backend").and_then(|v| v.as_str()) {
                Some("cuda")   => pb::GpuBackend::Cuda as i32,
                Some("metal")  => pb::GpuBackend::Metal as i32,
                Some("rocm")   => pb::GpuBackend::Rocm as i32,
                Some("vulkan") => pb::GpuBackend::Vulkan as i32,
                _              => pb::GpuBackend::Unspecified as i32,
            };
            let cap = g.get("capability").and_then(|v| v.as_object()).map(|c| pb::GpuCapabilityProfile {
                architecture:      c.get("architecture").and_then(|v| v.as_str()).unwrap_or_default().into(),
                compute_cap_major: c.get("compute_cap_major").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                compute_cap_minor: c.get("compute_cap_minor").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                vram_bytes:        c.get("vram_bytes").and_then(|v| v.as_u64()).unwrap_or(0),
                mem_bandwidth_gbs: c.get("mem_bandwidth_gbs").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                supports_fp16:     c.get("supports_fp16").and_then(|v| v.as_bool()).unwrap_or(false),
                supports_bf16:     c.get("supports_bf16").and_then(|v| v.as_bool()).unwrap_or(false),
                supports_fp8:      c.get("supports_fp8").and_then(|v| v.as_bool()).unwrap_or(false),
                supports_fp4:      c.get("supports_fp4").and_then(|v| v.as_bool()).unwrap_or(false),
                supports_int8_tc:  c.get("supports_int8_tc").and_then(|v| v.as_bool()).unwrap_or(false),
                benchmark:         None,
                backend:           backend_i,
                unified_memory:    c.get("unified_memory").and_then(|v| v.as_bool()).unwrap_or(false),
            });
            pb::GpuInfo {
                index:             g.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                uuid:              g.get("uuid").and_then(|v| v.as_str()).unwrap_or_default().into(),
                name:              g.get("name").and_then(|v| v.as_str()).unwrap_or_default().into(),
                architecture:      g.get("architecture").and_then(|v| v.as_str()).unwrap_or_default().into(),
                compute_cap_major: g.get("compute_cap_major").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                compute_cap_minor: g.get("compute_cap_minor").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                vram_total_bytes:  g.get("vram_total_bytes").and_then(|v| v.as_u64()).unwrap_or(0),
                vram_free_bytes:   g.get("vram_free_bytes").and_then(|v| v.as_u64()).unwrap_or(0),
                pci_bus_id:        g.get("pci_bus_id").and_then(|v| v.as_str()).unwrap_or_default().into(),
                driver_version:    g.get("driver_version").and_then(|v| v.as_str()).unwrap_or_default().into(),
                cuda_version:      g.get("cuda_version").and_then(|v| v.as_str()).unwrap_or_default().into(),
                vbios_version:     g.get("vbios_version").and_then(|v| v.as_str()).unwrap_or_default().into(),
                power_limit_w:     g.get("power_limit_w").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                nvlink_present:    g.get("nvlink_present").and_then(|v| v.as_bool()).unwrap_or(false),
                capability:        cap,
                backend:           backend_i,
                unified_memory:    g.get("unified_memory").and_then(|v| v.as_bool()).unwrap_or(false),
                gpu_core_count:    g.get("gpu_core_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                metal_family:      g.get("metal_family").and_then(|v| v.as_str()).unwrap_or_default().into(),
            }
        }).collect()
    }).unwrap_or_default();

    Ok(pb::NodeInfo {
        node_id:         s("node_id"),
        hostname:        s("hostname"),
        display_name:    s("display_name"),
        hw_fingerprint:  s("hw_fingerprint"),
        owner_user_id:   String::new(),
        tags:            arr_str("tags"),
        os,
        gpus,
        network,
        cpu_mem,
        geo:             None,
        status:          pb::NodeStatus::Online as i32,
        first_seen:      0,
        last_heartbeat:  0,
        agent_version:   s("agent_version"),
        client_cert_sha: String::new(),
    })
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
