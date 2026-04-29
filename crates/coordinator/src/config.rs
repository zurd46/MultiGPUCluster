#[derive(Debug, Clone)]
pub struct CoordConfig {
    pub grpc_bind: String,
    pub http_bind: String,
    pub database_url: Option<String>,
}
