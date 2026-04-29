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
    /// Optional mgmt-backend URL. When present + `mgmt_token` is set, /v1/models
    /// is sourced from the mgmt model registry (the source of truth the admin
    /// UI edits). When absent we fall back to a single "auto" entry derived
    /// from the live coordinator node count.
    pub mgmt_url: Option<String>,
    pub mgmt_token: Option<String>,
    pub http: reqwest::Client,
}

pub async fn run(
    bind: &str,
    coord: &str,
    mgmt_url: Option<String>,
    mgmt_token: Option<String>,
) -> Result<()> {
    let state = Arc::new(ApiState {
        coordinator_url: coord.trim_end_matches('/').to_string(),
        mgmt_url: mgmt_url.map(|s| s.trim_end_matches('/').to_string()),
        mgmt_token,
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
    // Source of truth: mgmt registry when configured, otherwise a synthetic
    // "auto" placeholder so LM Studio (which requires a non-empty model list)
    // doesn't refuse to render the connection.
    if let Some(rows) = fetch_registry(&s).await {
        if !rows.is_empty() {
            let data = rows
                .into_iter()
                .filter(|m| {
                    m.get("status")
                        .and_then(|v| v.as_str())
                        .map(|s| s != "disabled")
                        .unwrap_or(true)
                })
                .map(|m| ModelEntry {
                    id: m.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    object: "model",
                    owned_by: "gpucluster".into(),
                })
                .collect();
            return Json(ModelList {
                object: "list",
                data,
            });
        }
    }
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
    let _ = ChatResponse {
        id: String::new(),
        object: "chat.completion",
        created: 0,
        model: String::new(),
        choices: vec![Choice {
            index: 0,
            message: ChatMessage { role: "assistant".into(), content: String::new() },
            finish_reason: "stop".into(),
        }],
    };
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

/// Returns the model registry rows from mgmt-backend, or `None` if mgmt isn't
/// configured / unreachable. Caller falls back to the synthesised "auto" entry.
async fn fetch_registry(s: &ApiState) -> Option<Vec<serde_json::Value>> {
    let mgmt = s.mgmt_url.as_deref()?;
    let token = s.mgmt_token.as_deref()?;
    let url = format!("{mgmt}/api/v1/models");
    let res = s.http.get(url).bearer_auth(token).send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    res.json::<Vec<serde_json::Value>>().await.ok()
}
