//! POST /api/v1/enroll/tokens — admin-only, generates a one-time enrollment token.

use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use axum::{extract::State, Json};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Duration, Utc};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{api_error::{ApiError, ApiResult}, state::AppState};

#[derive(Debug, Deserialize, Default)]
pub struct CreateRequest {
    /// Optional human-readable hint (e.g. "workstation-dani") — purely informational.
    pub display_hint: Option<String>,
    /// Token TTL in seconds. Defaults to 900 (15 minutes).
    pub ttl_secs: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CreateResponse {
    pub token: String,
    pub expires_at: String,
}

pub async fn create(
    State(s): State<AppState>,
    Json(req): Json<CreateRequest>,
) -> ApiResult<Json<CreateResponse>> {
    let ttl = Duration::seconds(req.ttl_secs.unwrap_or(900).clamp(60, 24 * 3600));

    // Generate 32 bytes of randomness, base64-encode → user-facing token.
    let mut buf = [0u8; 32];
    SystemRandom::new()
        .fill(&mut buf)
        .map_err(|_| ApiError::internal("rng failed"))?;
    let token = URL_SAFE_NO_PAD.encode(buf);

    // Hash for storage. Argon2id gives us slow, salted comparisons even though
    // tokens are high-entropy already — defense in depth.
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    let hash = Argon2::default()
        .hash_password(token.as_bytes(), &salt)
        .map_err(|e| ApiError::internal(format!("argon2: {e}")))?
        .to_string();

    let id = Uuid::now_v7();
    let now = Utc::now();
    let expires = now + ttl;

    sqlx::query!(
        "INSERT INTO enroll_tokens (id, token_hash, display_hint, expires_at)
         VALUES ($1, $2, $3, $4)",
        id,
        hash,
        req.display_hint,
        expires,
    )
    .execute(&s.pool)
    .await?;

    sqlx::query!(
        "INSERT INTO audit_log (actor, action, resource, details)
         VALUES ('admin', 'ENROLL_TOKEN_GENERATED', $1, $2::jsonb)",
        id.to_string(),
        serde_json::json!({ "display_hint": req.display_hint, "ttl_secs": ttl.num_seconds() })
    )
    .execute(&s.pool)
    .await
    .ok();

    Ok(Json(CreateResponse {
        token,
        expires_at: expires.to_rfc3339(),
    }))
}
