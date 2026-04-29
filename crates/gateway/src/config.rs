#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub bind: String,
    pub mgmt_backend_url: String,
    pub coordinator_url: String,
    pub openai_api_url: String,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
    pub client_ca: Option<String>,
}
