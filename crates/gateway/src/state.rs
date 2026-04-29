use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Cached `/v1/*` auth verdict for a given customer token.
///
/// Argon2 verification on the mgmt-backend takes ~50 ms; without caching every
/// streaming token would round-trip an Argon2 hash. We cache the *positive*
/// result for 60 s and *negative* results for 5 s (so a freshly created key
/// becomes usable quickly, and a freshly revoked one stops working quickly).
#[derive(Clone, Debug)]
pub struct CachedAuth {
    pub ok: bool,
    pub key_id: Option<String>,
    pub name: Option<String>,
    pub scope: Option<String>,
    pub fetched_at: Instant,
}

impl CachedAuth {
    pub fn fresh(&self) -> bool {
        let ttl = if self.ok {
            Duration::from_secs(60)
        } else {
            Duration::from_secs(5)
        };
        self.fetched_at.elapsed() < ttl
    }
}

#[derive(Clone)]
pub struct GatewayState {
    pub mgmt_url: String,
    pub coordinator_http_url: String,
    pub openai_url: String,
    pub admin_api_key: Option<String>,
    pub http: reqwest::Client,
    /// Token → cached verdict. Keyed by the *raw* user token, which is fine
    /// because this map only lives in process memory and the keys are 32+
    /// bytes of entropy.
    pub auth_cache: Arc<DashMap<String, CachedAuth>>,
}

impl GatewayState {
    pub fn new(
        mgmt_url: String,
        coordinator_http_url: String,
        openai_url: String,
        admin_api_key: Option<String>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("build reqwest client");
        Self {
            mgmt_url,
            coordinator_http_url,
            openai_url,
            admin_api_key,
            http,
            auth_cache: Arc::new(DashMap::new()),
        }
    }
}
