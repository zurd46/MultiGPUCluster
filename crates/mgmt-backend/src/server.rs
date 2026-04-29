use anyhow::Result;
use axum::{
    middleware,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use std::{net::SocketAddr, sync::Arc};

use crate::{
    auth, ca_store,
    config::MgmtConfig,
    db,
    handlers::{audit, enroll, enroll_token, nodes},
    state::AppState,
};

pub async fn run(cfg: MgmtConfig) -> Result<()> {
    // 1) DB pool + migrations
    let pool = db::connect(&cfg.database_url).await?;
    db::migrate(&pool).await?;

    // 2) Root CA: load existing or generate fresh
    let ca = ca_store::load_or_init(&pool, &cfg.ca_common_name).await?;

    let state = AppState {
        pool,
        ca: Arc::new(ca),
        admin_api_key: cfg.admin_api_key.clone(),
        coordinator_endpoint: cfg.coordinator_endpoint.clone(),
    };

    // Public routes (worker-side and health)
    let public = Router::new()
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/api/v1/enroll", post(enroll::complete));

    // Admin routes (require ADMIN_API_KEY bearer)
    let admin = Router::new()
        .route("/api/v1/enroll/tokens",            post(enroll_token::create))
        .route("/api/v1/nodes",                    get(nodes::list))
        .route("/api/v1/nodes/{id}",               get(nodes::get))
        .route("/api/v1/nodes/{id}/approve",       post(nodes::approve))
        .route("/api/v1/nodes/{id}/revoke",        post(nodes::revoke))
        .route("/api/v1/nodes/{id}/drain",         post(nodes::drain))
        .route("/api/v1/audit",                    get(audit::list))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth::require_admin));

    let app = public.merge(admin).with_state(state);

    let addr: SocketAddr = cfg.bind.parse()?;
    tracing::info!(%addr, "mgmt-backend listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
