use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::{net::SocketAddr, sync::Arc};

use crate::schema::{ChatMessage, ChatRequest, ChatResponse, Choice, ModelEntry, ModelList};

#[derive(Clone)]
pub struct ApiState {
    pub coordinator_url: String,
    pub http: reqwest::Client,
}

pub async fn run(bind: &str, coord: &str) -> Result<()> {
    let state = Arc::new(ApiState {
        coordinator_url: coord.trim_end_matches('/').to_string(),
        http: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()?,
    });

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .with_state(state);

    let addr: SocketAddr = bind.parse()?;
    tracing::info!(%addr, coordinator = %coord, "openai-api listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn list_models(State(s): State<Arc<ApiState>>) -> Json<ModelList> {
    // Phase 2 will publish concrete model IDs once the model registry lands.
    // Until then we synthesize a single "auto" entry that always picks the
    // best-fit model for the current cluster — this keeps LM Studio happy
    // (it requires at least one model in the list) without lying about
    // capabilities.
    let cluster_size = probe_cluster_size(&s).await;
    Json(ModelList {
        object: "list",
        data: vec![ModelEntry {
            id: format!("auto (cluster: {} nodes)", cluster_size),
            object: "model",
            owned_by: "gpucluster".into(),
        }],
    })
}

async fn chat_completions(
    State(s): State<Arc<ApiState>>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, (StatusCode, Json<serde_json::Value>)> {
    // Phase 2 dispatcher (skeleton): ask the coordinator who can take work,
    // then return a structured response that names the chosen node. The
    // actual model invocation over llama.cpp RPC is the *next* commit; the
    // hop is the new hard part — once a node is reliably picked we can
    // forward the prompt to its `rpc-server-ext` and stream tokens back.
    let url = format!("{}/nodes/eligible", s.coordinator_url);
    let resp: serde_json::Value = match s.http.get(&url).send().await {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or(serde_json::Value::Null),
        Ok(r) => {
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": { "type": "coordinator_error", "status": r.status().as_u16() }
                })),
            ));
        }
        Err(e) => {
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": { "type": "coordinator_unreachable", "message": e.to_string() }
                })),
            ));
        }
    };

    let nodes = resp.get("nodes").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let Some(chosen) = nodes.first().cloned() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": {
                    "type": "no_eligible_nodes",
                    "message": "no worker is currently online with a usable GPU",
                    "request_model": req.model,
                }
            })),
        ));
    };

    // For now: structured 501 that *proves* dispatch picked a real node —
    // distinct from the previous blanket 503. The next step is to actually
    // forward to `chosen.wg_ip:50052` over llama.cpp's RPC protocol.
    Err((
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": {
                "type": "dispatch_not_wired",
                "message": "selected a worker but llama.cpp RPC forwarding is the next commit",
                "phase": "2-skeleton",
                "request_model": req.model,
                "request_messages": req.messages.len(),
                "chosen_worker": chosen,
            }
        })),
    ))
}

async fn probe_cluster_size(s: &ApiState) -> usize {
    let url = format!("{}/nodes", s.coordinator_url);
    match s.http.get(&url).send().await {
        Ok(r) if r.status().is_success() => {
            r.json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v.get("count").and_then(|n| n.as_u64()))
                .map(|n| n as usize)
                .unwrap_or(0)
        }
        _ => 0,
    }
}
