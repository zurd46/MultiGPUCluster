use anyhow::Result;
use axum::middleware as axmid;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use axum::http::StatusCode;
use tower_http::{
    cors::CorsLayer,
    limit::RequestBodyLimitLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::{config::GatewayConfig, middleware as gw, routes, state::GatewayState};

pub async fn run(cfg: GatewayConfig) -> Result<()> {
    let state = Arc::new(GatewayState::new(
        cfg.mgmt_backend_url.clone(),
        cfg.coordinator_url.clone(),
        cfg.openai_api_url.clone(),
        cfg.admin_api_key.clone(),
    ));

    let app = routes::build(state)
        .layer(axmid::from_fn(gw::capture_public_ip))
        .layer(axmid::from_fn(gw::request_id))
        .layer(CorsLayer::permissive())
        .layer(RequestBodyLimitLayer::new(50 * 1024 * 1024))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(300),
        ))
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = cfg.bind.parse()?;
    tracing::info!(%addr, "gateway listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
