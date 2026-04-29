//! POST /enroll  (gateway routes this from /enroll → mgmt:7100/api/v1/enroll)
//!
//! Worker sends one-time token + hardware fingerprint + pubkey.
//! We validate the token, register the node, and issue a short-lived mTLS cert.

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{
    extract::{ConnectInfo, State},
    http::HeaderMap,
    Json,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::ipnetwork::IpNetwork;
use std::net::SocketAddr;
use uuid::Uuid;

use crate::{api_error::{ApiError, ApiResult}, ca_store, state::AppState};

const NODE_CERT_VALID_DAYS: u32 = 7;

/// New worker payload: token + pubkey + the same `NodeInfo` JSON the worker
/// will keep heartbeating after enrollment. Keeping the inventory shape
/// identical to the heartbeat means the dashboard / DB sees one schema, and
/// enrollment isn't a special-case data path.
#[derive(Debug, Deserialize)]
pub struct EnrollRequest {
    pub token: String,
    pub pubkey_b64: String,
    #[serde(default)]
    pub display_name: Option<String>,
    /// Full `NodeInfo` JSON as produced by `gpucluster_sysinfo::inventory::to_json`.
    #[serde(default)]
    pub node: serde_json::Value,
}

impl EnrollRequest {
    fn node_str(&self, key: &str) -> String {
        self.node.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string()
    }
}

#[derive(Debug, Serialize)]
pub struct EnrollResponse {
    pub node_id: String,
    pub client_cert_pem: String,
    /// Private key the worker uses for mTLS. Sent over the (TLS-encrypted)
    /// enrollment channel exactly once; the bootstrapper persists it in
    /// `identity.json` mode 0600 and never transmits it again.
    pub client_key_pem: String,
    pub ca_chain_pem: String,
    pub coordinator_endpoint: String,
    pub cert_expires_at: String,
}

pub async fn complete(
    State(s): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<EnrollRequest>,
) -> ApiResult<Json<EnrollResponse>> {
    let hw_fingerprint = req.node_str("hw_fingerprint");
    let hostname       = req.node_str("hostname");
    let agent_version  = req.node_str("agent_version");
    let req_display    = req.display_name.clone()
        .or_else(|| Some(req.node_str("display_name")).filter(|s| !s.is_empty()));

    if req.token.is_empty() || hw_fingerprint.is_empty() || req.pubkey_b64.is_empty() {
        return Err(ApiError::BadRequest("token, pubkey_b64, node.hw_fingerprint required".into()));
    }

    // Behind a reverse proxy (Caddy → Gateway → mgmt), ConnectInfo gives us
    // the proxy IP, not the client. Trust X-Forwarded-For when present.
    let client_ip = client_ip(&headers).unwrap_or_else(|| addr.ip());

    // 1) Find a still-valid, unused enroll token whose hash matches the supplied token.
    let now = Utc::now();
    let candidates = sqlx::query!(
        "SELECT id, token_hash, display_hint
           FROM enroll_tokens
          WHERE used_at IS NULL
            AND expires_at > $1",
        now,
    )
    .fetch_all(&s.pool)
    .await?;

    let argon = Argon2::default();
    let matched = candidates.into_iter().find(|row| {
        PasswordHash::new(&row.token_hash)
            .ok()
            .map(|p| argon.verify_password(req.token.as_bytes(), &p).is_ok())
            .unwrap_or(false)
    });
    let token_row = matched.ok_or_else(|| ApiError::Unauthorized)?;

    // 2) Node already enrolled with same hardware fingerprint?  → re-issue cert,
    //    don't create a duplicate row.  (Re-enrollment after disk loss etc.)
    let existing = sqlx::query!(
        "SELECT id FROM nodes WHERE hw_fingerprint = $1",
        hw_fingerprint
    )
    .fetch_optional(&s.pool)
    .await?;

    let node_id = existing.map(|r| r.id).unwrap_or_else(Uuid::now_v7);

    // 3) Mint a short-lived mTLS client cert for this node, signed by the cluster CA.
    let issued = ca_store::issue_cert_for_node(&s.ca, &node_id.to_string(), NODE_CERT_VALID_DAYS)
        .map_err(ApiError::internal)?;
    let cert_expires = now + Duration::days(NODE_CERT_VALID_DAYS as i64);

    let public_ip: IpNetwork = match client_ip {
        std::net::IpAddr::V4(v) => format!("{v}/32").parse().unwrap(),
        std::net::IpAddr::V6(v) => format!("{v}/128").parse().unwrap(),
    };

    // 4) Persist node + token-used + audit, all in one transaction.
    let mut tx = s.pool.begin().await?;

    sqlx::query!(
        "INSERT INTO nodes (id, hw_fingerprint, hostname, display_name,
                            status, agent_version, client_cert_sha,
                            cert_expires_at,
                            current_public_ip_v4, public_ip_first_seen, public_ip_last_changed,
                            first_seen)
         VALUES ($1, $2, $3, $4, 'pending_approval', $5, $6, $7, $8, $9, $9, $9)
         ON CONFLICT (id) DO UPDATE SET
             hostname              = EXCLUDED.hostname,
             display_name          = COALESCE(EXCLUDED.display_name, nodes.display_name),
             agent_version         = EXCLUDED.agent_version,
             client_cert_sha       = EXCLUDED.client_cert_sha,
             cert_expires_at       = EXCLUDED.cert_expires_at,
             current_public_ip_v4  = EXCLUDED.current_public_ip_v4,
             public_ip_last_changed = EXCLUDED.public_ip_last_changed",
        node_id,
        hw_fingerprint,
        hostname,
        req_display.or(token_row.display_hint),
        agent_version,
        sha256_pem(&issued.cert_pem),
        cert_expires,
        public_ip,
        now,
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "INSERT INTO node_ip_history (node_id, public_ip_v4, source)
         VALUES ($1, $2, 'tls_socket')",
        node_id,
        public_ip,
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "UPDATE enroll_tokens
            SET used_at = now(), used_by_node = $1, used_from_ip = $2
          WHERE id = $3",
        node_id,
        public_ip,
        token_row.id,
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "INSERT INTO audit_log (actor, action, resource, ip, details)
         VALUES ('worker:enroll', 'NODE_ENROLLED', $1, $2, $3::jsonb)",
        node_id.to_string(),
        public_ip,
        serde_json::json!({
            "hw_fingerprint": hw_fingerprint,
            "hostname": hostname,
            "agent_version": agent_version,
            "device_name": req.node.get("os")
                .and_then(|o| o.get("device_name"))
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
            "gpus_count": req.node.get("gpus")
                .and_then(|g| g.as_array())
                .map(|a| a.len())
                .unwrap_or(0),
        })
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    tracing::info!(
        node_id = %node_id,
        hw = %hw_fingerprint,
        public_ip = %client_ip,
        gpus = req.node.get("gpus").and_then(|g| g.as_array()).map(|a| a.len()).unwrap_or(0),
        "node enrolled"
    );

    // Optional: hand the worker a WireGuard config (pre-auth key + server)
    // when the operator has wired up Headscale. Falls back to no-WG when
    // headscale is not configured — `INFERENCE_ADVERTISED_HOST` then stays
    // the dev workaround.
    let wg_config_ini = build_wireguard_config().await;

    Ok(Json(EnrollResponse {
        node_id: node_id.to_string(),
        client_cert_pem: issued.cert_pem,
        client_key_pem:  issued.key_pem,
        ca_chain_pem: s.ca.cert_pem(),
        coordinator_endpoint: s.coordinator_endpoint.clone(),
        cert_expires_at: cert_expires.to_rfc3339(),
        wg_config_ini,
    }))
}

/// Returns a join-config blob for the cluster's Headscale mesh when the
/// operator has wired it up (`HEADSCALE_URL` + `HEADSCALE_API_KEY` env), or
/// `None` otherwise. Best-effort: if Headscale is down we still return a
/// successful enrollment, just without WG. The coordinator's
/// `INFERENCE_ADVERTISED_HOST` escape hatch keeps dev usable.
async fn build_wireguard_config() -> Option<String> {
    let headscale_url = std::env::var("HEADSCALE_URL").ok().filter(|s| !s.is_empty())?;
    let api_key = std::env::var("HEADSCALE_API_KEY").ok().filter(|s| !s.is_empty())?;
    let user = std::env::var("HEADSCALE_USER").unwrap_or_else(|_| "cluster".into());

    let url = format!("{}/api/v1/preauthkey", headscale_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "user": user, "reusable": false, "ephemeral": false, "expiration": "1h",
    });
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;
    let resp = http.post(&url).bearer_auth(&api_key).json(&body).send().await.ok()?;
    if !resp.status().is_success() {
        tracing::warn!(status = %resp.status(), "headscale preauth key request failed");
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    let key = v.get("preAuthKey").and_then(|p| p.get("key")).and_then(|x| x.as_str())?;

    Some(format!(
        "# gpucluster Headscale join — bootstrapper writes this to a state\n\
         # file the worker's `tailscale up` reads. Real WG keys are managed\n\
         # by tailscaled.\n\
         ServerURL = {url}\n\
         AuthKey   = {key}\n\
         User      = {user}\n",
        url = headscale_url.trim_end_matches('/'),
    ))
}

/// Extract the originating client IP from X-Forwarded-For (left-most entry)
/// when the request has been routed through a trusted reverse proxy.
fn client_ip(headers: &HeaderMap) -> Option<std::net::IpAddr> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(str::trim)
        .and_then(|s| s.parse().ok())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
        })
}

fn sha256_pem(pem: &str) -> String {
    use ring::digest;
    let d = digest::digest(&digest::SHA256, pem.as_bytes());
    hex::encode(d.as_ref())
}

mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        const H: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(H[(b >> 4) as usize] as char);
            out.push(H[(b & 0x0f) as usize] as char);
        }
        out
    }
}
