pub mod enroll_token;
pub mod enroll;
pub mod nodes;
pub mod audit;
pub mod api_keys;

use serde::Serialize;

#[derive(Serialize)]
pub struct OkMessage {
    pub ok: bool,
}
