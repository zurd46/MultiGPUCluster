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
    // Real distributed inference lands in Phase 2 (llama.cpp RPC fork +
    // scheduler placement). Until then we return a structured 503 that tells
    // the caller exactly why no token came back, instead of pretending to
    // answer — silent stubs are worse than honest errors when wiring up
    // tooling like LM Studio.
    let nodes = probe_cluster_size(&s).await;
    let body = serde_json::json!({
        "error": {
            "type": "service_unavailable",
            "message": "distributed inference not yet wired up (Phase 2)",
            "phase": "1",
            "cluster_nodes_visible": nodes,
            "request_model": req.model,
            "request_messages": req.messages.len(),
            "next_step": "implement openai-api → scheduler → rpc-server-ext on workers"
        }
    });
    let _ = ChatResponse {
        // Keep the type alive so it compiles when we wire the real path; this
        // arm is unreachable today but flips on once the Phase-2 dispatcher
        // returns Result<ChatResponse, _>.
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
    Err((StatusCode::SERVICE_UNAVAILABLE, Json(body)))
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
