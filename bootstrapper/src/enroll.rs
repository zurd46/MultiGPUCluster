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

    let os_json = info.os.as_ref().map(|o| serde_json::json!({
        "family":  o.family,
        "version": o.version,
        "kernel":  o.kernel,
        "arch":    o.arch,
    })).unwrap_or(serde_json::Value::Null);

    let gpus_json: serde_json::Value = info.gpus.iter().map(|g| serde_json::json!({
        "index":             g.index,
        "uuid":              g.uuid,
        "name":              g.name,
        "architecture":      g.architecture,
        "compute_cap_major": g.compute_cap_major,
        "compute_cap_minor": g.compute_cap_minor,
        "vram_total_bytes":  g.vram_total_bytes,
        "driver_version":    g.driver_version,
        "cuda_version":      g.cuda_version,
        "vbios_version":     g.vbios_version,
    })).collect::<Vec<_>>().into();

    let payload = EnrollPayload {
        token,
        pubkey_b64: pub_b64,
        hw_fingerprint: info.hw_fingerprint.clone(),
        hostname: info.hostname.clone(),
        display_name,
        agent_version: env!("CARGO_PKG_VERSION"),
        os: os_json,
        gpus: gpus_json,
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
