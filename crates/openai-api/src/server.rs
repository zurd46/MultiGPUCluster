use anyhow::Result;
use axum::{routing::{get, post}, Json, Router};
use std::net::SocketAddr;

use crate::schema::{ChatRequest, ChatResponse, Choice, ChatMessage, ModelEntry, ModelList};

pub async fn run(bind: &str, _coord: &str) -> Result<()> {
    let app = Router::new()
        .route("/health",            get(|| async { "ok" }))
        .route("/v1/models",         get(list_models))
        .route("/v1/chat/completions", post(chat_completions));

    let addr: SocketAddr = bind.parse()?;
    tracing::info!(%addr, "openai-api listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn list_models() -> Json<ModelList> {
    Json(ModelList {
        object: "list",
        data: vec![ModelEntry {
            id: "stub-model".into(),
            object: "model",
            owned_by: "gpucluster".into(),
        }],
    })
}

async fn chat_completions(Json(req): Json<ChatRequest>) -> Json<ChatResponse> {
    Json(ChatResponse {
        id: format!("chatcmpl-{}", uuid::Uuid::now_v7()),
        object: "chat.completion",
        created: chrono::Utc::now().timestamp(),
        model: req.model,
        choices: vec![Choice {
            index: 0,
            message: ChatMessage {
                role: "assistant".into(),
                content: "Stub: connect cluster backend in Phase 2".into(),
            },
            finish_reason: "stop".into(),
        }],
    })
}
