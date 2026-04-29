//! Typed HTTP clients for cluster-internal service calls.
//!
//! Before this module each caller built its own URL with `format!("{base}/path/{id}")`,
//! its own `reqwest::Client`, and its own status-handling. That worked but
//! made it easy to:
//!   - typo a path and only find out at runtime,
//!   - forget to set a sensible timeout,
//!   - drift between services about how a 4xx error body should be parsed.
//!
//! These wrappers solve those one tier up: a `CoordClient::list_nodes()` call
//! is a single Rust function. Adding a new RPC means adding a method here
//! once, not editing N callers.
//!
//! What this is NOT: a generated client (no codegen, no OpenAPI dance). The
//! wire shape is whatever the handler returns; we pass it through as
//! `serde_json::Value` for endpoints that don't have stable structs yet,
//! and lift it to typed structs as the schemas settle. That keeps the
//! migration cheap.

use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;

/// Default timeout for cluster-internal calls. Tight enough that a wedged
/// peer doesn't stall callers; loose enough that a busy coordinator under
/// load still responds. Per-call overrides go through the builder methods
/// on the underlying `reqwest::Client`.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("upstream {service} returned {status}: {body}")]
    Upstream {
        service: &'static str,
        status: u16,
        body: String,
    },
    #[error("response not parseable as expected JSON: {0}")]
    Decode(String),
}

pub type ClientResult<T> = Result<T, ClientError>;

/// Build a `reqwest::Client` with the cluster's default timeouts. Callers
/// that need anything custom (large-file streaming, etc.) build their own
/// and pass it to a client constructor that takes one explicitly.
pub fn default_http_client() -> Client {
    Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// Build a worker-side `reqwest::Client` that presents the supplied client
/// certificate + key on every outbound TLS handshake (mTLS). The PEM blobs
/// are exactly what the enrollment response stored in `identity.json`.
///
/// `ca_chain_pem` is the cluster CA — used to *trust* the gateway's server
/// cert when it lives behind Caddy with the cluster's own CA, and on the
/// dev path we accept invalid certs so workers can talk to a Caddy
/// internally-issued cert without extra setup.
///
/// On any cert parse / build error we fall back to a non-mTLS client and log
/// a warning — heartbeat is more important than perfectly-secure transport
/// during dev. Production callers should treat the warning as fatal.
pub fn worker_http_client(
    client_cert_pem: &str,
    client_key_pem: &str,
    accept_invalid_certs: bool,
) -> Client {
    let mut builder = Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT);

    if !client_cert_pem.is_empty() && !client_key_pem.is_empty() {
        let combined = format!("{}\n{}", client_cert_pem.trim(), client_key_pem.trim());
        match reqwest::Identity::from_pem(combined.as_bytes()) {
            Ok(id) => {
                builder = builder.identity(id);
                tracing::info!("worker http client built with mTLS identity");
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse client identity PEM — falling back to plain HTTP");
            }
        }
    }
    if accept_invalid_certs {
        builder = builder.danger_accept_invalid_certs(true);
    }
    builder.build().unwrap_or_else(|_| Client::new())
}

// ---------- Coordinator ----------

/// Talks to the coordinator's HTTP API (default port [`super::ports::COORDINATOR_HTTP`]).
/// Used by mgmt-backend (for the load_model proxy + reconciler) and
/// openai-api (dispatcher).
#[derive(Clone)]
pub struct CoordClient {
    base: String,
    http: Client,
}

impl CoordClient {
    /// `base` is the HTTP root WITHOUT `/cluster` prefix — internal services
    /// call the coordinator directly (`http://coordinator:7001`), only the
    /// public gateway proxies `/cluster/*` here.
    pub fn new(base: impl Into<String>) -> Self {
        Self::with_http(base, default_http_client())
    }
    pub fn with_http(base: impl Into<String>, http: Client) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            http,
        }
    }
    pub fn base(&self) -> &str {
        &self.base
    }

    /// `GET /nodes` — full registry view including offline + duplicate-rejected.
    pub async fn list_nodes(&self) -> ClientResult<Value> {
        json_get(&self.http, "coordinator", &format!("{}/nodes", self.base)).await
    }

    /// `GET /nodes/eligible` — slim list of nodes that can take inference work.
    pub async fn eligible_nodes(&self) -> ClientResult<Value> {
        json_get(
            &self.http,
            "coordinator",
            &format!("{}/nodes/eligible", self.base),
        )
        .await
    }

    /// `POST /nodes/{id}/load_model` — proxied to the worker's control plane.
    /// `body` is forwarded verbatim. Returns whatever the worker (via the
    /// proxy) sent back so callers can surface the worker's ack message.
    pub async fn load_model(&self, node_id: &str, body: &Value) -> ClientResult<Value> {
        json_post(
            &self.http,
            "coordinator",
            &format!("{}/nodes/{}/load_model", self.base, node_id),
            body,
        )
        .await
    }
}

// ---------- mgmt-backend ----------

/// Talks to the mgmt-backend's admin API. Used by openai-api (model registry
/// sync + inference log writes).
#[derive(Clone)]
pub struct MgmtClient {
    base: String,
    bearer: Option<String>,
    http: Client,
}

impl MgmtClient {
    pub fn new(base: impl Into<String>, bearer: Option<String>) -> Self {
        Self::with_http(base, bearer, default_http_client())
    }
    pub fn with_http(base: impl Into<String>, bearer: Option<String>, http: Client) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            bearer,
            http,
        }
    }
    pub fn has_token(&self) -> bool {
        self.bearer.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
    }

    /// `GET /api/v1/models` — returns the registry rows the admin UI edits.
    /// openai-api uses this to populate `/v1/models`.
    pub async fn list_models(&self) -> ClientResult<Vec<Value>> {
        let url = format!("{}/api/v1/models", self.base);
        let req = self.http.get(&url);
        let req = if let Some(t) = &self.bearer {
            req.bearer_auth(t)
        } else {
            req
        };
        let resp = req.send().await?;
        decode_or_upstream(resp, "mgmt").await
    }

    /// `POST /api/v1/inference/log` — fire-and-forget request log entry from
    /// openai-api. We swallow transport errors at the call site since logging
    /// failures must never affect the customer's request path.
    pub async fn log_inference(&self, body: &Value) -> ClientResult<()> {
        let url = format!("{}/api/v1/inference/log", self.base);
        let req = self.http.post(&url).json(body);
        let req = if let Some(t) = &self.bearer {
            req.bearer_auth(t)
        } else {
            req
        };
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(ClientError::Upstream {
                service: "mgmt",
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        Ok(())
    }
}

// ---------- worker (single-node llama-server forward) ----------

/// Talks to a worker's `llama-server` HTTP endpoint. The dispatcher uses
/// this for non-streaming `/v1/chat/completions` forwarding; streaming has
/// its own bytes-passthrough path (kept inline because it owns the response
/// body lifecycle directly).
#[derive(Clone)]
pub struct WorkerClient {
    base: String,
    http: Client,
}

impl WorkerClient {
    pub fn new(base: impl Into<String>) -> Self {
        Self::with_http(base, default_http_client())
    }
    pub fn with_http(base: impl Into<String>, http: Client) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            http,
        }
    }

    /// `POST <worker>/v1/chat/completions` — forward the customer's request
    /// to llama-server. Returns the parsed JSON body on success. Streaming
    /// requests should bypass this and use the raw `bytes_stream()`.
    pub async fn chat_completions(&self, body: &Value) -> ClientResult<Value> {
        json_post(
            &self.http,
            "worker",
            &format!("{}/v1/chat/completions", self.base),
            body,
        )
        .await
    }
}

// ---------- worker control plane ----------

/// Talks to a worker's control endpoint (default port
/// [`super::ports::WORKER_CONTROL`]) — handles `load_model` and other
/// admin-initiated commands. Distinct from [`WorkerClient`] because the
/// control plane and inference plane listen on different ports and have
/// different threat models (inference can be public; control is admin-only).
#[derive(Clone)]
pub struct WorkerControlClient {
    base: String,
    http: Client,
}

impl WorkerControlClient {
    pub fn new(base: impl Into<String>) -> Self {
        Self::with_http(base, default_http_client())
    }
    pub fn with_http(base: impl Into<String>, http: Client) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            http,
        }
    }

    /// `POST /control/load_model` — request the worker download a GGUF from
    /// HuggingFace and respawn its llama-server against it. The worker
    /// returns 202 immediately; actual progress shows up in the heartbeat.
    pub async fn load_model(&self, body: &Value) -> ClientResult<Value> {
        json_post(
            &self.http,
            "worker_control",
            &format!("{}/control/load_model", self.base),
            body,
        )
        .await
    }
}

// ---------- helpers ----------

async fn json_get(http: &Client, service: &'static str, url: &str) -> ClientResult<Value> {
    let resp = http.get(url).send().await?;
    decode_or_upstream(resp, service).await
}

async fn json_post(
    http: &Client,
    service: &'static str,
    url: &str,
    body: &Value,
) -> ClientResult<Value> {
    let resp = http.post(url).json(body).send().await?;
    decode_or_upstream(resp, service).await
}

async fn decode_or_upstream<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
    service: &'static str,
) -> ClientResult<T> {
    let status = resp.status();
    if !status.is_success() {
        return Err(ClientError::Upstream {
            service,
            status: status.as_u16(),
            body: resp.text().await.unwrap_or_default(),
        });
    }
    resp.json::<T>()
        .await
        .map_err(|e| ClientError::Decode(e.to_string()))
}
