//! /api/v1/keys — admin-minted API keys for /v1/* (OpenAI-compatible inference).
//!
//! Token format: `mgc_<32-byte url-safe base64>`. The `mgc_` prefix is purely
//! cosmetic — it lets users spot a leaked key at a glance and lets us reject
//! malformed tokens without an Argon2 round-trip. The first 12 characters of
//! the *full* token (so `mgc_<8 chars>`) are stored as `prefix` for the admin
//! UI listing.
//!
//! Storage: Argon2id over the full token. Even though the entropy alone makes
//! offline attacks infeasible, the slow hash gives defence-in-depth and keeps
//! the storage shape consistent with `enroll_tokens`.
//!
//! Verification ([`verify`]) is called by the gateway (admin-bearer-protected)
//! on every /v1/* request — but the gateway caches the result, so the Argon2
//! cost is amortised.

use argon2::{
    password_hash::{PasswordHash, PasswordVerifier, SaltString},
    Argon2, PasswordHasher,
};
use axum::{
    extract::{Path, State},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Duration, Utc};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    api_error::{ApiError, ApiResult},
    state::AppState,
};

const TOKEN_PREFIX: &str = "mgc_";
/// Length of the user-visible token prefix we store for UI listings —
/// `mgc_` (4) + 8 characters of base64 = 12. Long enough to disambiguate
/// dozens of keys at a glance, short enough to be useless for brute force.
const STORED_PREFIX_LEN: usize = 12;

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    /// Human-readable label, e.g. "lm-studio-laptop". Required.
    pub name: String,
    /// `inference` (default) for /v1/* access, `admin` for full mgmt access.
    /// `admin` keys are reserved for future use — the gateway only checks
    /// scope==`inference`|`admin` for /v1/* today.
    #[serde(default)]
    pub scope: Option<String>,
    /// Optional TTL in seconds. Omitted = no expiry.
    #[serde(default)]
    pub ttl_secs: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CreateResponse {
    pub id: Uuid,
    /// The full token — shown ONCE at creation time, never retrievable again.
    pub token: String,
    pub prefix: String,
    pub name: String,
    pub scope: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct KeyRow {
    pub id: Uuid,
    pub name: String,
    pub prefix: String,
    pub scope: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

pub async fn create(
    State(s): State<AppState>,
    Json(req): Json<CreateRequest>,
) -> ApiResult<Json<CreateResponse>> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name must not be empty".into()));
    }
    let scope = req.scope.as_deref().unwrap_or("inference").to_string();
    if !matches!(scope.as_str(), "inference" | "admin") {
        return Err(ApiError::BadRequest(
            "scope must be 'inference' or 'admin'".into(),
        ));
    }

    // 32 bytes of randomness → ~256 bits of entropy. Brute-force impossible.
    let mut buf = [0u8; 32];
    SystemRandom::new()
        .fill(&mut buf)
        .map_err(|_| ApiError::internal("rng failed"))?;
    let random = URL_SAFE_NO_PAD.encode(buf);
    let token = format!("{TOKEN_PREFIX}{random}");

    // Stored prefix is the first STORED_PREFIX_LEN chars of the *full* token,
    // i.e. includes "mgc_". `random` is ASCII base64-url so byte-slicing is safe.
    let prefix = token.chars().take(STORED_PREFIX_LEN).collect::<String>();

    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    let hash = Argon2::default()
        .hash_password(token.as_bytes(), &salt)
        .map_err(|e| ApiError::internal(format!("argon2: {e}")))?
        .to_string();

    let id = Uuid::now_v7();
    let expires_at = req
        .ttl_secs
        .filter(|n| *n > 0)
        .map(|n| Utc::now() + Duration::seconds(n.clamp(60, 365 * 24 * 3600)));

    sqlx::query!(
        "INSERT INTO api_keys (id, user_id, hash, scope, name, prefix, expires_at)
         VALUES ($1, NULL, $2, $3, $4, $5, $6)",
        id,
        hash,
        scope,
        name,
        prefix,
        expires_at,
    )
    .execute(&s.pool)
    .await?;

    sqlx::query!(
        "INSERT INTO audit_log (actor, action, resource, details)
         VALUES ('admin', 'API_KEY_CREATED', $1, $2::jsonb)",
        id.to_string(),
        serde_json::json!({ "name": name, "scope": scope, "prefix": prefix }),
    )
    .execute(&s.pool)
    .await
    .ok();

    Ok(Json(CreateResponse {
        id,
        token,
        prefix,
        name: name.to_string(),
        scope,
        expires_at: expires_at.map(|t| t.to_rfc3339()),
    }))
}

pub async fn list(State(s): State<AppState>) -> ApiResult<Json<Vec<KeyRow>>> {
    let rows = sqlx::query_as!(
        KeyRow,
        r#"SELECT id, name, prefix, scope, created_at, last_used_at, expires_at, revoked_at
           FROM api_keys
           ORDER BY created_at DESC
           LIMIT 500"#,
    )
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(rows))
}

pub async fn revoke(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let res = sqlx::query!(
        "UPDATE api_keys SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL",
        id
    )
    .execute(&s.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    sqlx::query!(
        "INSERT INTO audit_log (actor, action, resource)
         VALUES ('admin', 'API_KEY_REVOKED', $1)",
        id.to_string(),
    )
    .execute(&s.pool)
    .await
    .ok();
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct VerifyRequest {
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyResponse {
    pub ok: bool,
    pub key_id: Option<Uuid>,
    pub name: Option<String>,
    pub scope: Option<String>,
}

/// POST /api/v1/keys/verify — used by the gateway to authenticate /v1/* calls.
///
/// Protected by the same admin-bearer middleware as the rest of /api/v1/*,
/// because only the gateway (which holds ADMIN_API_KEY for service-to-service
/// auth) is allowed to ask "is this customer token valid?". This avoids
/// turning the verify endpoint into an open Argon2 oracle.
pub async fn verify(
    State(s): State<AppState>,
    Json(req): Json<VerifyRequest>,
) -> ApiResult<Json<VerifyResponse>> {
    let token = req.token.trim();
    if !token.starts_with(TOKEN_PREFIX) || token.len() < STORED_PREFIX_LEN + 4 {
        return Ok(Json(VerifyResponse {
            ok: false,
            key_id: None,
            name: None,
            scope: None,
        }));
    }
    let prefix: String = token.chars().take(STORED_PREFIX_LEN).collect();

    // Indexed lookup by prefix narrows the candidate set to ~1 row in practice
    // (collision probability is 1 in 2^48 over 8 base64 chars). Then Argon2
    // verify on each candidate.
    let candidates = sqlx::query!(
        r#"SELECT id, hash, name, scope, expires_at, revoked_at
           FROM api_keys WHERE prefix = $1"#,
        prefix
    )
    .fetch_all(&s.pool)
    .await?;

    let now = Utc::now();
    for c in candidates {
        if c.revoked_at.is_some() {
            continue;
        }
        if c.expires_at.map(|t| t < now).unwrap_or(false) {
            continue;
        }
        let parsed = match PasswordHash::new(&c.hash) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if Argon2::default()
            .verify_password(token.as_bytes(), &parsed)
            .is_ok()
        {
            // Best-effort touch — don't fail verification if it errors.
            let _ = sqlx::query!(
                "UPDATE api_keys SET last_used_at = now() WHERE id = $1",
                c.id
            )
            .execute(&s.pool)
            .await;
            return Ok(Json(VerifyResponse {
                ok: true,
                key_id: Some(c.id),
                name: Some(c.name),
                scope: Some(c.scope),
            }));
        }
    }

    Ok(Json(VerifyResponse {
        ok: false,
        key_id: None,
        name: None,
        scope: None,
    }))
}
