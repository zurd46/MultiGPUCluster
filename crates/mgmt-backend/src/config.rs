#[derive(Debug, Clone)]
pub struct MgmtConfig {
    pub bind: String,
    pub database_url: String,
    pub jwt_secret: String,
    pub admin_api_key: String,
    pub coordinator_endpoint: String,
    pub ca_common_name: String,
}
