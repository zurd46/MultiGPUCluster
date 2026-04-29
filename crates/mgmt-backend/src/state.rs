use gpucluster_ca::Ca;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub ca: Arc<Ca>,
    pub admin_api_key: String,
    pub coordinator_endpoint: String,
}
