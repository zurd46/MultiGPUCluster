#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub bind: String,
    pub mgmt_backend_url: String,
    /// HTTP base URL of the coordinator (port 7001), not the gRPC bind.
    pub coordinator_url: String,
    pub openai_api_url: String,
    pub admin_api_key: Option<String>,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
    pub client_ca: Option<String>,
}
