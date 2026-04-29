use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EnrollToken {
    pub id: Uuid,
    pub token: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub used: bool,
}

pub fn issue(ttl: Duration) -> EnrollToken {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let mut buf = [0u8; 32];
    use ring::rand::SecureRandom;
    let rng = ring::rand::SystemRandom::new();
    rng.fill(&mut buf).expect("rng");
    let now = Utc::now();
    EnrollToken {
        id: Uuid::now_v7(),
        token: URL_SAFE_NO_PAD.encode(buf),
        created_at: now,
        expires_at: now + ttl,
        used: false,
    }
}

pub fn validate(token: &EnrollToken) -> Result<()> {
    if token.used {
        anyhow::bail!("token already used");
    }
    if token.expires_at < Utc::now() {
        anyhow::bail!("token expired");
    }
    Ok(())
}
