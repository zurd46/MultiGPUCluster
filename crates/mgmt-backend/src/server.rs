use anyhow::Result;
use axum::{routing::{get, post}, Router, Json};
use serde_json::json;
use std::net::SocketAddr;

use crate::config::MgmtConfig;

pub async fn run(cfg: MgmtConfig) -> Result<()> {
    let app = Router::new()
        .route("/health",                          get(|| async { Json(json!({"status":"ok"})) }))
        .route("/api/v1/auth/login",               post(stub))
        .route("/api/v1/users",                    get(stub))
        .route("/api/v1/nodes",                    get(stub))
        .route("/api/v1/nodes/{id}",               get(stub))
        .route("/api/v1/nodes/{id}/approve",       post(stub))
        .route("/api/v1/nodes/{id}/revoke",        post(stub))
        .route("/api/v1/nodes/{id}/drain",         post(stub))
        .route("/api/v1/nodes/{id}/ip-history",    get(stub))
        .route("/api/v1/enroll/tokens",            post(stub))
        .route("/api/v1/enroll",                   post(stub))
        .route("/api/v1/jobs",                     get(stub))
        .route("/api/v1/jobs/{id}",                get(stub))
        .route("/api/v1/audit",                    get(stub));

    let addr: SocketAddr = cfg.bind.parse()?;
    tracing::info!(%addr, "mgmt-backend listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn stub() -> Json<serde_json::Value> {
    Json(json!({"todo": "phase 1+ implementation"}))
}
