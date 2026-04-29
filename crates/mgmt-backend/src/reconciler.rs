//! Background loop that reconciles the `models` table against what workers
//! actually report in their heartbeats.
//!
//! Why this exists:
//!   The model registry has a `loaded_on_node` column. Today, two writers
//!   touch it:
//!     1. The Load handler (mgmt-backend POST /api/v1/models/{id}/load) sets
//!        it optimistically the moment the admin clicks Load.
//!     2. The worker's heartbeat reports `current_model = X` once the file
//!        is on disk and `llama-server` is running.
//!
//!   Without a reconciler the two drift: a worker can crash mid-download,
//!   the optimistic write stays forever; or a model finishes loading on a
//!   different worker than the one we asked. The reconciler is the single
//!   source of truth — it walks the coordinator's live registry every 5 s
//!   and writes the *observed* state back to the DB.
//!
//! Status state machine driven from here:
//!     downloading | loading  ── worker reports current_model ─▶  available
//!                              ── stuck > STUCK_THRESHOLD     ─▶  error
//!     available                ── worker stops reporting it  ─▶  available
//!                                                                (loaded_on_node cleared,
//!                                                                 row stays advertised on /v1/models)
//!     disabled  ── never touched (admin-controlled)
//!     error     ── never touched (admin retries by clicking Load)
//!
//! The reconciler never deletes rows and never touches `disabled`/`error`.

use chrono::{DateTime, Utc};
use gpucluster_common::clients::CoordClient;
use sqlx::PgPool;
use std::collections::HashMap;
use std::time::Duration;

const RECONCILE_PERIOD: Duration = Duration::from_secs(5);
/// How long a model can sit in `downloading`/`loading` before we conclude the
/// worker died and flip the row to `error`. 5 min covers the worst-case
/// download of an ~5 GB GGUF on a slow link; smaller models finish in seconds.
const STUCK_THRESHOLD: Duration = Duration::from_secs(300);

pub fn spawn(pool: PgPool, coordinator_endpoint: String) {
    if coordinator_endpoint.is_empty() {
        tracing::warn!("reconciler disabled: no coordinator_endpoint configured");
        return;
    }
    tokio::spawn(async move {
        let coord = CoordClient::new(coordinator_endpoint);
        let mut tick = tokio::time::interval(RECONCILE_PERIOD);
        // Discard the immediate first tick so we don't race with mgmt-backend
        // startup (the coordinator may not be reachable yet on cold boot).
        tick.tick().await;
        loop {
            tick.tick().await;
            if let Err(e) = reconcile_once(&coord, &pool).await {
                tracing::warn!(error = %e, "reconcile pass failed");
            }
        }
    });
}

async fn reconcile_once(coord: &CoordClient, pool: &PgPool) -> anyhow::Result<()> {
    let body = coord.list_nodes().await?;
    let nodes = body
        .get("nodes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // model_id -> first node_id we observed serving it.
    // "First" because sorting comes from the coordinator's registry order;
    // if multiple workers somehow both report the same model_id (e.g. after
    // a manual MODEL_PATH spawn collides with a load_model), the later one
    // is treated as a duplicate and not represented here. The admin UI's
    // "loaded on" column will show one node — that's a deliberate
    // simplification, the dispatcher already picks any eligible node.
    let mut loaded: HashMap<String, String> = HashMap::new();
    for n in &nodes {
        let node_id = n.get("node_id").and_then(|v| v.as_str()).unwrap_or("");
        let model = n
            .get("current_model")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if node_id.is_empty() || model.is_empty() {
            continue;
        }
        loaded.entry(model.to_string()).or_insert_with(|| node_id.to_string());
    }

    let rows = sqlx::query!(
        r#"SELECT id, status, loaded_on_node, updated_at
           FROM models
           WHERE status NOT IN ('disabled', 'error')"#
    )
    .fetch_all(pool)
    .await?;

    for r in rows {
        let observed = loaded.get(&r.id).cloned();
        let in_flight = matches!(r.status.as_str(), "downloading" | "loading");

        // Decide the new (loaded_on_node, status) pair.
        let (new_node, new_status) = match (observed.as_deref(), in_flight) {
            // Worker confirms the load → flip to available.
            (Some(node), true) => (node.to_string(), "available".to_string()),
            // Worker confirms an already-available model → just keep node fresh.
            (Some(node), false) => (node.to_string(), r.status.clone()),
            // Nobody reports it AND status is in-flight: check stuckness.
            (None, true) => {
                if is_stuck(&r.updated_at) {
                    ("".to_string(), "error".to_string())
                } else {
                    // Still legitimately downloading — leave optimistic write alone.
                    (r.loaded_on_node.clone(), r.status.clone())
                }
            }
            // Nobody reports it, status is `available`: clear the node hint.
            // The model row stays in the registry (admins can re-Load it),
            // we just stop pretending it's loaded somewhere.
            (None, false) => ("".to_string(), r.status.clone()),
        };

        if new_node != r.loaded_on_node || new_status != r.status {
            sqlx::query!(
                r#"UPDATE models
                      SET loaded_on_node = $2,
                          status         = $3,
                          updated_at     = now()
                    WHERE id = $1"#,
                r.id,
                new_node,
                new_status,
            )
            .execute(pool)
            .await?;
            sqlx::query!(
                "INSERT INTO audit_log (actor, action, resource, details)
                 VALUES ('reconciler', 'MODEL_RECONCILED', $1, $2::jsonb)",
                r.id,
                serde_json::json!({
                    "previous_node":   r.loaded_on_node,
                    "current_node":    new_node,
                    "previous_status": r.status,
                    "current_status":  new_status,
                }),
            )
            .execute(pool)
            .await
            .ok();
            tracing::info!(
                model = %r.id,
                old_node = %r.loaded_on_node,
                new_node = %new_node,
                old_status = %r.status,
                new_status = %new_status,
                "model state reconciled"
            );
        }
    }
    Ok(())
}

fn is_stuck(since: &DateTime<Utc>) -> bool {
    Utc::now()
        .signed_duration_since(*since)
        .to_std()
        .map(|d| d > STUCK_THRESHOLD)
        .unwrap_or(false)
}
