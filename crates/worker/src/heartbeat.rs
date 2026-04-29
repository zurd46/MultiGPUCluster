//! Periodic upload of the full node inventory to the gateway.
//!
//! Behaviour:
//!   - On startup we POST the snapshot once to `/nodes/report` so the gateway
//!     learns about the host immediately (no waiting for the first tick).
//!   - Then every 30s we POST a refreshed snapshot (vram_free, cpu/ram, public
//!     IP all change over time). The endpoint is upsert-by-node_id so there
//!     is no separate "register" vs "heartbeat" call.
//!   - Failures are logged but never fatal — the worker keeps trying. This
//!     covers both backend maintenance windows and ISP IP flips.
//!
//! The gateway proxies `/cluster/*` to the coordinator's HTTP listener (see
//! `crates/gateway/src/routes.rs::cluster_proxy`). Workers therefore hit
//! `{coordinator_url}/nodes/report` where `coordinator_url` is either the
//! gateway URL with `/cluster` already appended (production) or the
//! coordinator's HTTP listener directly (dev).
//!
//! In addition to inventory we now ship two fields the coordinator wires up
//! into its registry:
//!   - `inference_endpoint`: `:port` of the llama-server, when one is loaded.
//!   - `control_endpoint`:   `:port` of the worker's control plane (always).
//!   - `current_model`:      logical id of the loaded model, when any.

use crate::inference_server::SharedSupervisor;
use gpucluster_proto::node as pb;
use gpucluster_sysinfo::inventory;
use std::time::Duration;
use tokio::time::interval;

const HEARTBEAT_PERIOD: Duration = Duration::from_secs(30);

pub async fn run_loop(
    coordinator_url: String,
    mut info: pb::NodeInfo,
    sup: SharedSupervisor,
    control_endpoint: String,
) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let report_url = format!("{}/nodes/report", coordinator_url.trim_end_matches('/'));
    let node_id = info.node_id.clone();

    // Initial publish — happens before the first interval tick so the
    // gateway has the inventory within milliseconds of worker start.
    publish(&client, &report_url, &info, &sup, &control_endpoint).await;

    let mut tick = interval(HEARTBEAT_PERIOD);
    tick.tick().await; // consume the immediate first tick
    loop {
        tick.tick().await;

        // Refresh volatile fields (free VRAM, free RAM, public IP). Identity
        // fields stay frozen — node_id, hostname, hw_fingerprint never change
        // between reboots.
        match gpucluster_sysinfo::collect() {
            Ok(mut fresh) => {
                fresh.node_id = node_id.clone();
                fresh.display_name = info.display_name.clone();
                info = fresh;
            }
            Err(e) => tracing::warn!(error = %e, "sysinfo refresh failed; reusing last snapshot"),
        }
        publish(&client, &report_url, &info, &sup, &control_endpoint).await;
    }
}

async fn publish(
    client: &reqwest::Client,
    url: &str,
    info: &pb::NodeInfo,
    sup: &SharedSupervisor,
    control_endpoint: &str,
) {
    // Snapshot the supervisor state under the lock, then drop it before the
    // (potentially slow) HTTP call so a stuck network doesn't stall the
    // control server.
    let (inference_endpoint, current_model) = {
        let guard = sup.lock().await;
        (guard.endpoint_advertise(), guard.model_id.clone())
    };

    let mut body = inventory::to_json(info);
    if let serde_json::Value::Object(map) = &mut body {
        if let Some(ep) = inference_endpoint.as_deref() {
            // Worker reports `:50053` (port-only); coordinator pairs it with the
            // observed public/wg IP to build a fully-qualified URL.
            map.insert(
                "inference_endpoint".into(),
                serde_json::Value::String(ep.to_string()),
            );
        }
        // control_endpoint is always advertised — it's up regardless of
        // whether a model is currently loaded, so the coordinator can route
        // a load_model RPC to it.
        map.insert(
            "control_endpoint".into(),
            serde_json::Value::String(control_endpoint.to_string()),
        );
        if let Some(m) = current_model.as_deref() {
            map.insert(
                "current_model".into(),
                serde_json::Value::String(m.to_string()),
            );
        }
    }
    match client.post(url).json(&body).send().await {
        Ok(r) => {
            if r.status().is_success() {
                tracing::debug!(
                    node = %info.node_id,
                    gpus = info.gpus.len(),
                    model = current_model.as_deref().unwrap_or("none"),
                    "inventory published",
                );
            } else {
                tracing::warn!(node = %info.node_id, status = %r.status(), "inventory upload rejected");
            }
        }
        Err(e) => tracing::warn!(error = %e, %url, "inventory upload failed"),
    }
}
