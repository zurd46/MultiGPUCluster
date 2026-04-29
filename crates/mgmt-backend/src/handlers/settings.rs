//! /api/v1/settings — cluster-wide configuration that the admin UI edits.
//!
//! Backed by the `cluster_settings` KV table (TEXT key, JSONB value). The
//! handler exposes the whole settings object as a single document so the UI
//! can do "fetch → edit → PUT" without juggling per-key requests. Unknown
//! keys round-trip untouched, which keeps forward-compat painless when we
//! add new fields.

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    api_error::{ApiError, ApiResult},
    state::AppState,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsDoc(pub Map<String, Value>);

pub async fn get(State(s): State<AppState>) -> ApiResult<Json<SettingsDoc>> {
    let rows = sqlx::query!("SELECT key, value FROM cluster_settings")
        .fetch_all(&s.pool)
        .await?;
    let mut map = Map::new();
    for r in rows {
        map.insert(r.key, r.value);
    }
    Ok(Json(SettingsDoc(map)))
}

pub async fn put(
    State(s): State<AppState>,
    Json(doc): Json<SettingsDoc>,
) -> ApiResult<Json<SettingsDoc>> {
    if doc.0.is_empty() {
        return Err(ApiError::BadRequest("body must not be empty".into()));
    }
    // Whitelist what the admin UI is allowed to touch. Strict allowlist
    // because a typo'd key name would otherwise create a phantom setting
    // that nothing reads.
    const ALLOWED: &[&str] = &[
        "public_base_url",
        "default_model",
        "rate_limit_rpm",
        "max_tokens_default",
        // Hugging Face access token used by workers when downloading gated/private
        // GGUFs. Empty string is a valid value: it disables HF auth and only
        // public repos remain reachable. We store the raw token in JSONB rather
        // than hashed because the worker needs the cleartext to call the Hub —
        // mitigations: the field is admin-only (require_admin middleware on
        // every read/write) and never logged.
        "huggingface_api_token",
    ];

    let mut tx = s.pool.begin().await?;
    for (k, v) in doc.0.iter() {
        if !ALLOWED.contains(&k.as_str()) {
            return Err(ApiError::BadRequest(format!("unknown setting: {k}")));
        }
        sqlx::query!(
            r#"INSERT INTO cluster_settings (key, value, updated_at)
               VALUES ($1, $2, now())
               ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()"#,
            k,
            v
        )
        .execute(&mut *tx)
        .await?;
    }
    // Redact secrets before writing them to the audit log. The audit log is
    // surfaced verbatim by GET /api/v1/audit, so any value placed here is
    // recoverable by anyone with admin access. Token-shaped fields get hashed
    // out and only their presence is recorded.
    let mut redacted = doc.0.clone();
    if let Some(v) = redacted.get_mut("huggingface_api_token") {
        let was_set = v.as_str().map(|s| !s.is_empty()).unwrap_or(false);
        *v = Value::String(if was_set { "<redacted>".into() } else { "<cleared>".into() });
    }
    sqlx::query!(
        "INSERT INTO audit_log (actor, action, resource, details)
         VALUES ('admin', 'SETTINGS_UPDATED', 'cluster_settings', $1::jsonb)",
        Value::Object(redacted),
    )
    .execute(&mut *tx)
    .await
    .ok();
    tx.commit().await?;
    get(State(s)).await
}
