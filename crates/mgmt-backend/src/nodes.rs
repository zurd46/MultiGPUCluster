use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRecord {
    pub id: Uuid,
    pub display_name: String,
    pub hw_fingerprint: String,
    pub owner_user_id: Option<Uuid>,
    pub status: String,
    pub current_public_ip_v4: Option<String>,
    pub current_country: Option<String>,
}
