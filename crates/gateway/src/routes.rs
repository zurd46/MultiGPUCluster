use axum::{routing::{get, post}, Router, Json};
use serde_json::json;

pub fn build() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready",  get(ready))
        .route("/enroll", post(enroll))
        .nest("/v1",     openai_routes())
        .nest("/api",    mgmt_routes())
        .nest("/cluster", cluster_routes())
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({"status": "ok"}))
}

async fn ready() -> Json<serde_json::Value> {
    Json(json!({"ready": true}))
}

async fn enroll() -> Json<serde_json::Value> {
    Json(json!({"todo": "proxy to mgmt-backend /enroll"}))
}

fn openai_routes() -> Router {
    Router::new()
        .route("/chat/completions", post(stub))
        .route("/completions",      post(stub))
        .route("/models",           get(stub))
}

fn mgmt_routes() -> Router {
    Router::new()
        .route("/nodes",            get(stub))
        .route("/users",            get(stub))
        .route("/jobs",             get(stub))
        .route("/audit",            get(stub))
}

fn cluster_routes() -> Router {
    Router::new()
        .route("/heartbeat",        post(stub))
        .route("/jobs/poll",        get(stub))
}

async fn stub() -> Json<serde_json::Value> {
    Json(json!({"todo": "phase 1+ implementation"}))
}
