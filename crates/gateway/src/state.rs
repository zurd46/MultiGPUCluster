use std::time::Duration;

#[derive(Clone)]
pub struct GatewayState {
    pub mgmt_url: String,
    pub coordinator_http_url: String,
    pub openai_url: String,
    pub admin_api_key: Option<String>,
    pub http: reqwest::Client,
}

impl GatewayState {
    pub fn new(
        mgmt_url: String,
        coordinator_http_url: String,
        openai_url: String,
        admin_api_key: Option<String>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("build reqwest client");
        Self {
            mgmt_url,
            coordinator_http_url,
            openai_url,
            admin_api_key,
            http,
        }
    }
}
