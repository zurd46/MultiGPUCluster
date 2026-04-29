use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub node_id: String,
    pub signing_key_b64: String,
    pub client_cert_pem: String,
    pub ca_chain_pem: String,
    pub wg_config_ini: Option<String>,
    pub coordinator_endpoint: String,
}

pub fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("NODE_DATA_DIR") {
        return PathBuf::from(p);
    }
    if cfg!(windows) {
        PathBuf::from(r"C:\ProgramData\gpucluster")
    } else {
        PathBuf::from("/var/lib/gpucluster")
    }
}

pub fn identity_path() -> PathBuf {
    data_dir().join("identity.json")
}

pub fn persist_identity(id: Identity) -> Result<()> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = identity_path();
    let json = serde_json::to_vec_pretty(&id)?;
    write_secure(&path, &json)?;
    Ok(())
}

pub fn load_identity() -> Result<Option<Identity>> {
    let path = identity_path();
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    Ok(Some(serde_json::from_slice(&data)?))
}

#[cfg(unix)]
fn write_secure(path: &Path, data: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(data)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_secure(path: &Path, data: &[u8]) -> Result<()> {
    std::fs::write(path, data)?;
    Ok(())
}
