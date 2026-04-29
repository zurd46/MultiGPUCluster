pub mod enroll_token;
pub mod enroll;
pub mod nodes;
pub mod audit;

use serde::Serialize;

#[derive(Serialize)]
pub struct OkMessage {
    pub ok: bool,
}
