//! /api/v1/models — model registry. Drives what `/v1/models` advertises to
//! OpenAI-compatible clients (LM Studio etc.).
//!
//! Phase 1 stores the metadata only. Phase 2 will sync the `status` column
//! from live worker state — until then it's whatever the admin set.

use axum::{
    extract::{Path, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub is_default: Option<bool>,
}

fn validate_status(s: &str) -> ApiResult<()> {
    if matches!(s, "available" | "loading" | "disabled" | "error") {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "status must be one of: available, loading, disabled, error".into(),
        ))
    }
}

pub async fn list(State(s): State<AppState>) -> ApiResult<Json<Vec<ModelRow>>> {
    let rows = sqlx::query_as!(
        ModelRow,
        r#"SELECT id, display_name, description, status, is_default, created_at, updated_at
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

    let mut tx = s.pool.begin().await?;
    if req.is_default {
        sqlx::query!("UPDATE models SET is_default = FALSE WHERE is_default = TRUE")
            .execute(&mut *tx)
            .await?;
    }
    let row = sqlx::query_as!(
        ModelRow,
        r#"INSERT INTO models (id, display_name, description, status, is_default)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, display_name, description, status, is_default, created_at, updated_at"#,
        id,
        req.display_name.unwrap_or_default(),
        req.description.unwrap_or_default(),
        status,
        req.is_default,
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
              display_name = COALESCE($2, display_name),
              description  = COALESCE($3, description),
              status       = COALESCE($4, status),
              is_default   = COALESCE($5, is_default),
              updated_at   = now()
           WHERE id = $1
           RETURNING id, display_name, description, status, is_default, created_at, updated_at"#,
        id,
        req.display_name.as_deref(),
        req.description.as_deref(),
        req.status.as_deref(),
        req.is_default,
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
