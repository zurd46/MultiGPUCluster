use anyhow::{Context, Result};
use gpucluster_ca::{generate_root_ca, issue_node_cert, Ca, IssuedCert};
use rcgen::{CertificateParams, KeyPair};
use sqlx::PgPool;

/// Get or initialise the cluster's Root CA.
/// First call generates fresh CA materials and persists them; subsequent
/// calls re-hydrate the in-memory `Ca` from the DB so we can sign new certs.
pub async fn load_or_init(pool: &PgPool, common_name: &str) -> Result<Ca> {
    let row = sqlx::query!(
        "SELECT cert_pem, key_pem FROM ca_state WHERE id = 1"
    )
    .fetch_optional(pool)
    .await
    .context("query ca_state")?;

    if let Some(r) = row {
        let key = KeyPair::from_pem(&r.key_pem).context("parse stored CA key")?;
        let params = CertificateParams::from_ca_cert_pem(&r.cert_pem)
            .context("parse stored CA cert")?;
        let cert = params.self_signed(&key).context("rebuild CA cert")?;
        tracing::info!(common_name = %common_name, "loaded existing root CA");
        return Ok(Ca { cert, key });
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
