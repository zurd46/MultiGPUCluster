use anyhow::{Context, Result};
use gpucluster_ca::{generate_root_ca, issue_node_cert, load_ca, Ca, IssuedCert};
use sqlx::PgPool;

/// Get or initialise the cluster's Root CA.
/// First call generates fresh CA materials and persists them; subsequent
/// calls re-hydrate the in-memory `Ca` from the DB so we can sign new certs.
pub async fn load_or_init(pool: &PgPool, common_name: &str) -> Result<Ca> {
    let row = sqlx::query!(
        "SELECT common_name, cert_pem, key_pem FROM ca_state WHERE id = 1"
    )
    .fetch_optional(pool)
    .await
    .context("query ca_state")?;

    if let Some(r) = row {
        let ca = load_ca(&r.common_name, &r.cert_pem, &r.key_pem)
            .context("rehydrate stored CA")?;
        tracing::info!(common_name = %r.common_name, "loaded existing root CA");
        return Ok(ca);
    }

    tracing::warn!(common_name = %common_name, "no CA in DB — generating new root CA");
    let ca = generate_root_ca(common_name)?;
    sqlx::query!(
        "INSERT INTO ca_state (id, common_name, cert_pem, key_pem)
         VALUES (1, $1, $2, $3)",
        common_name,
        ca.cert_pem(),
        ca.key_pem(),
    )
    .execute(pool)
    .await
    .context("persist new CA")?;
    Ok(ca)
}

pub fn issue_cert_for_node(ca: &Ca, node_id: &str, valid_days: u32) -> Result<IssuedCert> {
    issue_node_cert(ca, node_id, valid_days)
}
