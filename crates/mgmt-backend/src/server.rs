use anyhow::Result;
use axum::{
    middleware,
    routing::{get, patch, post},
    Json, Router,
};
use serde_json::json;
use std::{net::SocketAddr, sync::Arc};

use crate::{
    auth, ca_store,
    config::MgmtConfig,
    db,
    handlers::{api_keys, audit, enroll, enroll_token, inference, models, nodes, settings},
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
        // Customer API keys for /v1/*. `verify` is admin-bearer-only too —
        // it's a service-to-service call from the gateway, not a public route.
        // PATCH edits metadata (name/scope/ttl); DELETE soft-revokes by default
        // and hard-purges with ?purge=true; POST /{id}/revoke is the explicit
        // soft-revoke for clients that don't speak DELETE.
        .route("/api/v1/keys",                     post(api_keys::create).get(api_keys::list))
        .route("/api/v1/keys/{id}",
               patch(api_keys::update).delete(api_keys::delete))
        .route("/api/v1/keys/{id}/revoke",         post(api_keys::revoke))
        .route("/api/v1/keys/verify",              post(api_keys::verify))
        // Cluster-wide settings (KV) and the model registry. Both feed the
        // OpenAI-API + the admin UI.
        .route("/api/v1/settings",                 get(settings::get).put(settings::put))
        .route("/api/v1/models",                   get(models::list).post(models::create))
        .route("/api/v1/models/{id}",
               patch(models::update).delete(models::delete))
        // Trigger an HF download + llama-server restart on the chosen worker.
        // Async: the worker downloads in the background and reports the new
        // status on its next heartbeat.
        .route("/api/v1/models/{id}/load",         post(models::load))
        // Per-request inference log. POST is service-to-service (openai-api
        // writes one row per /v1/chat/completions); GET is the read view the
        // admin UI consumes.
        .route("/api/v1/inference/log",            post(inference::log))
        .route("/api/v1/inference/recent",         get(inference::recent))
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
