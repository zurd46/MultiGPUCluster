use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub ts: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub resource: Option<String>,
    pub ip: Option<String>,
    pub details: serde_json::Value,
}

pub fn entry(actor: &str, action: &str) -> AuditEntry {
    AuditEntry {
        id: Uuid::now_v7(),
        ts: Utc::now(),
        actor: actor.to_string(),
        action: action.to_string(),
        resource: None,
        ip: None,
        details: serde_json::json!({}),
    }
}
