//! /api/v1/inference — request log written by the openai-api dispatcher,
//! surfaced read-only by the admin UI.
//!
//! Two routes:
//!   POST /api/v1/inference/log     — service-to-service write (openai-api)
//!   GET  /api/v1/inference/recent  — admin read with pagination + filters
//!
//! Both sit behind the existing admin-bearer middleware. openai-api uses the
//! same `ADMIN_API_KEY` we already share for the model registry sync, so no
//! new credential to manage.

use axum::{
    extract::{Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{api_error::ApiResult, state::AppState};

#[derive(Debug, Deserialize)]
pub struct LogRequest {
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub inference_url: Option<String>,
    #[serde(default)]
    pub api_key_prefix: Option<String>,
    #[serde(default)]
    pub prompt_tokens: Option<i32>,
    #[serde(default)]
    pub completion_tokens: Option<i32>,
    #[serde(default)]
    pub total_tokens: Option<i32>,
    #[serde(default)]
    pub latency_ms: Option<i32>,
    pub status_code: i32,
    #[serde(default)]
    pub error_type: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
}

fn default_endpoint() -> String {
    "chat.completions".to_string()
}

#[derive(Debug, Serialize)]
pub struct LogRow {
    pub id: uuid::Uuid,
    pub request_id: Option<String>,
    pub endpoint: String,
    pub model: String,
    pub node_id: Option<String>,
    pub inference_url: Option<String>,
    pub api_key_prefix: Option<String>,
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
    pub latency_ms: Option<i32>,
    pub status_code: i32,
    pub error_type: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn log(
    State(s): State<AppState>,
    Json(req): Json<LogRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    // Cap error_message — llama-server can return multi-MB error bodies and
    // we don't want to balloon the table. 4 KB is plenty for a stack trace
    // or a 502 body.
    let truncated_message = req
        .error_message
        .map(|s| if s.len() > 4096 { s[..4096].to_string() } else { s });

    sqlx::query!(
        r#"INSERT INTO inference_log (
              request_id, endpoint, model, node_id, inference_url,
              api_key_prefix, prompt_tokens, completion_tokens, total_tokens,
              latency_ms, status_code, error_type, error_message
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#,
        req.request_id,
        req.endpoint,
        req.model,
        req.node_id,
        req.inference_url,
        req.api_key_prefix,
        req.prompt_tokens,
        req.completion_tokens,
        req.total_tokens,
        req.latency_ms,
        req.status_code,
        req.error_type,
        truncated_message,
    )
    .execute(&s.pool)
    .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct RecentQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// "ok" → only successes. "errors" → only failures. Default = both.
    #[serde(default)]
    pub status: Option<String>,
}

pub async fn recent(
    State(s): State<AppState>,
    Query(q): Query<RecentQuery>,
) -> ApiResult<Json<Vec<LogRow>>> {
    // Hard cap so a curious admin can't pull a million rows by mistake.
    let limit = q.limit.unwrap_or(200).clamp(1, 1000);

    let only_errors = matches!(q.status.as_deref(), Some("errors"));
    let only_ok = matches!(q.status.as_deref(), Some("ok"));
    let node = q.node_id.filter(|s| !s.is_empty());
    let model = q.model.filter(|s| !s.is_empty());

    // sqlx's compile-time checker doesn't combine well with dynamic WHERE
    // clauses, so we build the row set with pushdown via filtering NULLs:
    // each predicate becomes "param is NULL OR column matches". Cheap on
    // small result sets and keeps the query macro happy.
    let rows = sqlx::query_as!(
        LogRow,
        r#"
        SELECT id, request_id, endpoint, model, node_id, inference_url,
               api_key_prefix, prompt_tokens, completion_tokens, total_tokens,
               latency_ms, status_code, error_type, error_message, created_at
        FROM inference_log
        WHERE ($1::text IS NULL OR node_id = $1)
          AND ($2::text IS NULL OR model   = $2)
          AND ($3::bool IS FALSE OR error_type IS NOT NULL)
          AND ($4::bool IS FALSE OR error_type IS NULL)
        ORDER BY created_at DESC
        LIMIT $5
        "#,
        node,
        model,
        only_errors,
        only_ok,
        limit,
    )
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(rows))
}
