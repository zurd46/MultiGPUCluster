//! GET /api/v1/audit  — admin-only read-only view of the immutable audit log.

use axum::{
    extract::{Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::ipnetwork::IpNetwork;
use uuid::Uuid;

use crate::{api_error::ApiResult, state::AppState};

#[derive(Debug, Deserialize)]
pub struct ListParams {
    /// Max rows to return (1..=500, default 100). The audit log is append-only
    /// so a small page-size keeps the admin UI responsive even on busy clusters.
    #[serde(default)]
    pub limit: Option<i64>,
    /// Filter by exact action name, e.g. `NODE_ENROLLED`, `NODE_REVOKED`.
    #[serde(default)]
    pub action: Option<String>,
    /// Filter by actor (user id, "admin", "worker:enroll", …).
    #[serde(default)]
    pub actor: Option<String>,
    /// Filter by resource (typically a node UUID stringified).
    #[serde(default)]
    pub resource: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuditRow {
    pub id: Uuid,
    pub ts: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub resource: Option<String>,
    pub ip: Option<String>,
    pub details: serde_json::Value,
}

pub async fn list(
    State(s): State<AppState>,
    Query(p): Query<ListParams>,
) -> ApiResult<Json<Vec<AuditRow>>> {
    let limit = p.limit.unwrap_or(100).clamp(1, 500);

    let rows = sqlx::query!(
        r#"SELECT id, ts, actor, action, resource, ip, details
             FROM audit_log
            WHERE ($1::text IS NULL OR action   = $1)
              AND ($2::text IS NULL OR actor    = $2)
              AND ($3::text IS NULL OR resource = $3)
            ORDER BY ts DESC
            LIMIT $4"#,
        p.action.as_deref(),
        p.actor.as_deref(),
        p.resource.as_deref(),
        limit,
    )
    .fetch_all(&s.pool)
    .await?;

    let out = rows
        .into_iter()
        .map(|r| AuditRow {
            id: r.id,
            ts: r.ts,
            actor: r.actor,
            action: r.action,
            resource: r.resource,
            ip: r.ip.map(stringify_ip),
            details: r.details,
        })
        .collect();
    Ok(Json(out))
}

fn stringify_ip(ip: IpNetwork) -> String {
    ip.ip().to_string()
}
