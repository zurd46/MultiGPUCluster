use axum::{extract::{Path, State}, Json};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::{api_error::{ApiError, ApiResult}, state::AppState};

#[derive(Debug, Serialize)]
pub struct NodeRow {
    pub id: Uuid,
    pub hostname: Option<String>,
    pub display_name: Option<String>,
    pub status: String,
    pub agent_version: Option<String>,
    pub current_public_ip_v4: Option<String>,
    pub current_country: Option<String>,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_heartbeat: Option<DateTime<Utc>>,
    pub cert_expires_at: Option<DateTime<Utc>>,
}

pub async fn list(State(s): State<AppState>) -> ApiResult<Json<Vec<NodeRow>>> {
    let rows = sqlx::query_as!(
        NodeRow,
        r#"SELECT id,
                  hostname,
                  display_name,
                  status,
                  agent_version,
                  host(current_public_ip_v4) AS current_public_ip_v4,
                  current_country,
                  first_seen,
                  last_heartbeat,
                  cert_expires_at
           FROM nodes
           ORDER BY first_seen DESC NULLS LAST
           LIMIT 200"#,
    )
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(rows))
}

pub async fn get(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<NodeRow>> {
    let row = sqlx::query_as!(
        NodeRow,
        r#"SELECT id,
                  hostname,
                  display_name,
                  status,
                  agent_version,
                  host(current_public_ip_v4) AS current_public_ip_v4,
                  current_country,
                  first_seen,
                  last_heartbeat,
                  cert_expires_at
           FROM nodes WHERE id = $1"#,
        id
    )
    .fetch_optional(&s.pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(row))
}

pub async fn approve(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let res = sqlx::query!(
        "UPDATE nodes SET status = 'online'
          WHERE id = $1 AND status = 'pending_approval'",
        id
    )
    .execute(&s.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    sqlx::query!(
        "INSERT INTO audit_log (actor, action, resource)
         VALUES ('admin', 'NODE_APPROVED', $1)",
        id.to_string(),
    )
    .execute(&s.pool)
    .await
    .ok();
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn revoke(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    sqlx::query!("UPDATE nodes SET status = 'revoked' WHERE id = $1", id)
        .execute(&s.pool)
        .await?;
    sqlx::query!(
        "INSERT INTO audit_log (actor, action, resource)
         VALUES ('admin', 'NODE_REVOKED', $1)",
        id.to_string(),
    )
    .execute(&s.pool)
    .await
    .ok();
    Ok(Json(serde_json::json!({ "ok": true })))
}
