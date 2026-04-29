//! /api/v1/models — model registry. Drives what `/v1/models` advertises to
//! OpenAI-compatible clients (LM Studio etc.).
//!
//! Two flavours of model row coexist:
//!
//!   1. Manual-path models (legacy / dev): admin sets `id`, `display_name`,
//!      and points a worker's `MODEL_PATH` at the file by hand. `hf_repo` is
//!      empty.
//!   2. HuggingFace-sourced models: admin fills `hf_repo` + `hf_file`, then
//!      hits POST /api/v1/models/{id}/load?node_id=… which dispatches a
//!      download+restart to the chosen worker (via the coordinator's
//!      `/nodes/{id}/load_model` proxy).
//!
//! Status transitions for HF models:
//!     available  → downloading → loading → available
//!                              ↘ error    (download or spawn failed)

use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    api_error::{ApiError, ApiResult},
    state::AppState,
};

#[derive(Debug, Serialize)]
pub struct ModelRow {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub status: String,
    pub is_default: bool,
    pub hf_repo: String,
    pub hf_file: String,
    pub local_filename: String,
    pub loaded_on_node: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub hf_repo: Option<String>,
    #[serde(default)]
    pub hf_file: Option<String>,
    #[serde(default)]
    pub local_filename: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub is_default: Option<bool>,
    pub hf_repo: Option<String>,
    pub hf_file: Option<String>,
    pub local_filename: Option<String>,
}

fn validate_status(s: &str) -> ApiResult<()> {
    if matches!(
        s,
        "available" | "loading" | "downloading" | "disabled" | "error"
    ) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "status must be one of: available, loading, downloading, disabled, error".into(),
        ))
    }
}

pub async fn list(State(s): State<AppState>) -> ApiResult<Json<Vec<ModelRow>>> {
    let rows = sqlx::query_as!(
        ModelRow,
        r#"SELECT id, display_name, description, status, is_default,
                  hf_repo, hf_file, local_filename, loaded_on_node,
                  created_at, updated_at
           FROM models
           ORDER BY is_default DESC, created_at DESC"#,
    )
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(rows))
}

pub async fn create(
    State(s): State<AppState>,
    Json(req): Json<CreateRequest>,
) -> ApiResult<Json<ModelRow>> {
    let id = req.id.trim();
    if id.is_empty() {
        return Err(ApiError::BadRequest("id must not be empty".into()));
    }
    let status = req.status.unwrap_or_else(|| "available".to_string());
    validate_status(&status)?;

    let hf_repo = req.hf_repo.unwrap_or_default();
    let hf_file = req.hf_file.unwrap_or_default();
    if !hf_repo.is_empty() && hf_file.is_empty() {
        return Err(ApiError::BadRequest(
            "hf_file is required when hf_repo is set".into(),
        ));
    }
    // Default the on-disk filename to the HF file basename so the admin
    // doesn't have to repeat themselves. Stays empty for non-HF models.
    let local_filename = req
        .local_filename
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| hf_file.clone());

    let mut tx = s.pool.begin().await?;
    if req.is_default {
        sqlx::query!("UPDATE models SET is_default = FALSE WHERE is_default = TRUE")
            .execute(&mut *tx)
            .await?;
    }
    let row = sqlx::query_as!(
        ModelRow,
        r#"INSERT INTO models (id, display_name, description, status, is_default,
                                hf_repo, hf_file, local_filename)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING id, display_name, description, status, is_default,
                     hf_repo, hf_file, local_filename, loaded_on_node,
                     created_at, updated_at"#,
        id,
        req.display_name.unwrap_or_default(),
        req.description.unwrap_or_default(),
        status,
        req.is_default,
        hf_repo,
        hf_file,
        local_filename,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            ApiError::Conflict(format!("model '{id}' already exists"))
        }
        e => ApiError::internal(e),
    })?;

    sqlx::query!(
        "INSERT INTO audit_log (actor, action, resource)
         VALUES ('admin', 'MODEL_CREATED', $1)",
        id,
    )
    .execute(&mut *tx)
    .await
    .ok();
    tx.commit().await?;

    Ok(Json(row))
}

pub async fn update(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRequest>,
) -> ApiResult<Json<ModelRow>> {
    if let Some(status) = req.status.as_deref() {
        validate_status(status)?;
    }
    let mut tx = s.pool.begin().await?;
    if req.is_default == Some(true) {
        sqlx::query!(
            "UPDATE models SET is_default = FALSE WHERE is_default = TRUE AND id <> $1",
            id
        )
        .execute(&mut *tx)
        .await?;
    }
    let row = sqlx::query_as!(
        ModelRow,
        r#"UPDATE models SET
              display_name   = COALESCE($2, display_name),
              description    = COALESCE($3, description),
              status         = COALESCE($4, status),
              is_default     = COALESCE($5, is_default),
              hf_repo        = COALESCE($6, hf_repo),
              hf_file        = COALESCE($7, hf_file),
              local_filename = COALESCE($8, local_filename),
              updated_at     = now()
           WHERE id = $1
           RETURNING id, display_name, description, status, is_default,
                     hf_repo, hf_file, local_filename, loaded_on_node,
                     created_at, updated_at"#,
        id,
        req.display_name.as_deref(),
        req.description.as_deref(),
        req.status.as_deref(),
        req.is_default,
        req.hf_repo.as_deref(),
        req.hf_file.as_deref(),
        req.local_filename.as_deref(),
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;

    sqlx::query!(
        "INSERT INTO audit_log (actor, action, resource)
         VALUES ('admin', 'MODEL_UPDATED', $1)",
        id,
    )
    .execute(&mut *tx)
    .await
    .ok();
    tx.commit().await?;
    Ok(Json(row))
}

pub async fn delete(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let res = sqlx::query!("DELETE FROM models WHERE id = $1", id)
        .execute(&s.pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    sqlx::query!(
        "INSERT INTO audit_log (actor, action, resource)
         VALUES ('admin', 'MODEL_DELETED', $1)",
        id,
    )
    .execute(&s.pool)
    .await
    .ok();
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct LoadQuery {
    /// Worker that should download + run this model. Required — there's no
    /// useful "auto-pick a node" for now (the scheduler is Phase 3).
    pub node_id: String,
}

/// POST /api/v1/models/{id}/load?node_id=…
///
/// Tells the chosen worker to fetch this model from Hugging Face and restart
/// its local llama-server. The actual download is async on the worker; we
/// flip the row to `downloading` and return 202 immediately. The worker's
/// next heartbeat will report `current_model = id` (or `error`).
pub async fn load(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<LoadQuery>,
) -> ApiResult<Json<Value>> {
    let model = sqlx::query!(
        r#"SELECT id, hf_repo, hf_file, local_filename
           FROM models WHERE id = $1"#,
        id
    )
    .fetch_optional(&s.pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    if model.hf_repo.is_empty() || model.hf_file.is_empty() {
        return Err(ApiError::BadRequest(
            "model has no hf_repo/hf_file — set them before calling load".into(),
        ));
    }
    let local_filename = if model.local_filename.is_empty() {
        model.hf_file.clone()
    } else {
        model.local_filename.clone()
    };

    // HF token is optional — public repos don't need one. Empty string here
    // means "send no Authorization header on the HF request".
    let token: String = sqlx::query_scalar!(
        r#"SELECT value FROM cluster_settings WHERE key = 'huggingface_api_token'"#
    )
    .fetch_optional(&s.pool)
    .await?
    .and_then(|v| v.as_str().map(String::from))
    .unwrap_or_default();

    // Forward to coordinator's load_model proxy. The coordinator knows which
    // IP+port the worker's control endpoint is on (it sees them on each
    // heartbeat); it does the actual TCP hop.
    // node_ids are UUIDs (see worker/src/identity.rs). Reject anything else
    // before stitching it into a URL path — keeps us out of percent-encoding
    // territory and rejects obvious injection attempts.
    if !q.node_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(ApiError::BadRequest("node_id must be a UUID".into()));
    }
    let coord = s.coordinator_endpoint.trim_end_matches('/');
    let url = format!("{}/nodes/{}/load_model", coord, q.node_id);
    let body = json!({
        "model_id":       id,
        "hf_repo":        model.hf_repo,
        "hf_file":        model.hf_file,
        "hf_token":       token,
        "local_filename": local_filename,
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(ApiError::internal)?;
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("coordinator unreachable: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(ApiError::Internal(format!(
            "coordinator rejected load (status={status}): {text}"
        )));
    }

    // Optimistic UI: flip status + remember which node we asked. Worker
    // heartbeat will overwrite these once the download completes (or fails).
    sqlx::query!(
        r#"UPDATE models
              SET status = 'downloading',
                  loaded_on_node = $2,
                  updated_at = now()
            WHERE id = $1"#,
        id,
        q.node_id,
    )
    .execute(&s.pool)
    .await?;
    sqlx::query!(
        "INSERT INTO audit_log (actor, action, resource, details)
         VALUES ('admin', 'MODEL_LOAD_REQUESTED', $1, $2::jsonb)",
        id,
        json!({ "node_id": q.node_id }),
    )
    .execute(&s.pool)
    .await
    .ok();

    Ok(Json(json!({
        "ok": true,
        "model_id": id,
        "node_id":  q.node_id,
        "status":   "downloading",
    })))
}
