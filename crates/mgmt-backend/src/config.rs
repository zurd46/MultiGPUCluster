#[derive(Debug, Clone)]
pub struct MgmtConfig {
    pub bind: String,
    pub database_url: Option<String>,
    pub jwt_secret: String,
}
