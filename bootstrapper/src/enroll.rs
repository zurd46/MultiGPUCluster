use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::state;

#[derive(Serialize)]
struct EnrollPayload<'a> {
    token: &'a str,
    pubkey_b64: String,
    hw_fingerprint: String,
    hostname: String,
    display_name: Option<&'a str>,
    agent_version: &'a str,
    os: serde_json::Value,
    gpus: serde_json::Value,
}

#[derive(Deserialize)]
struct EnrollResponse {
    node_id: String,
    client_cert_pem: String,
    ca_chain_pem: String,
    #[serde(default)]
    wg_config_ini: Option<String>,
    coordinator_endpoint: String,
}

pub async fn run(backend: &str, token: &str, display_name: Option<&str>) -> Result<()> {
    let info = gpucluster_sysinfo::collect()?;
    let (priv_b64, pub_b64) = generate_ed25519_keypair()?;

    let payload = EnrollPayload {
        token,
        pubkey_b64: pub_b64,
        hw_fingerprint: info.hw_fingerprint.clone(),
        hostname: info.hostname.clone(),
        display_name,
        agent_version: env!("CARGO_PKG_VERSION"),
        os: serde_json::to_value(&info.os).unwrap_or_default(),
        gpus: serde_json::to_value(&info.gpus).unwrap_or_default(),
    };

    let url = format!("{}/enroll", backend.trim_end_matches('/'));
    tracing::info!(%url, "submitting enrollment");

    let client = reqwest::Client::builder().build()?;
    let resp = client.post(&url).json(&payload).send().await
        .context("enrollment request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("enroll failed: {} {}", status, body);
    }

    let resp: EnrollResponse = resp.json().await.context("parse enroll response")?;

    state::persist_identity(state::Identity {
        node_id: resp.node_id,
        signing_key_b64: priv_b64,
        client_cert_pem: resp.client_cert_pem,
        ca_chain_pem: resp.ca_chain_pem,
        wg_config_ini: resp.wg_config_ini,
        coordinator_endpoint: resp.coordinator_endpoint,
    })?;

    tracing::info!("enrollment successful — identity persisted");
    Ok(())
}

/// Generates a fresh Ed25519 keypair using the system RNG (via ring).
/// Returns (private_key_b64, public_key_b64) where bytes are the raw
/// 32-byte secret seed and 32-byte public key.
fn generate_ed25519_keypair() -> Result<(String, String)> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use ring::rand::{SecureRandom, SystemRandom};
    use ring::signature::{Ed25519KeyPair, KeyPair};

    let rng = SystemRandom::new();
    let mut seed = [0u8; 32];
    rng.fill(&mut seed).map_err(|_| anyhow::anyhow!("rng failed"))?;

    let kp = Ed25519KeyPair::from_seed_unchecked(&seed)
        .map_err(|e| anyhow::anyhow!("ed25519 keygen: {e}"))?;
    let pub_b64 = STANDARD.encode(kp.public_key().as_ref());
    let priv_b64 = STANDARD.encode(seed);
    Ok((priv_b64, pub_b64))
}
