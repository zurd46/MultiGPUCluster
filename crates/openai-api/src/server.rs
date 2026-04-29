use anyhow::Result;
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use std::{net::SocketAddr, sync::Arc, time::Instant};

use crate::schema::{ModelEntry, ModelList};

#[derive(Clone)]
pub struct ApiState {
    pub coordinator_url: String,
    /// Optional mgmt-backend URL. When present + `mgmt_token` is set, /v1/models
    /// is sourced from the mgmt model registry (the source of truth the admin
    /// UI edits). When absent we fall back to a single "auto" entry derived
    /// from the live coordinator node count.
    pub mgmt_url: Option<String>,
    pub mgmt_token: Option<String>,
    pub http: reqwest::Client,
}

pub async fn run(
    bind: &str,
    coord: &str,
    mgmt_url: Option<String>,
    mgmt_token: Option<String>,
) -> Result<()> {
    let state = Arc::new(ApiState {
        coordinator_url: coord.trim_end_matches('/').to_string(),
        mgmt_url: mgmt_url.map(|s| s.trim_end_matches('/').to_string()),
        mgmt_token,
        http: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()?,
    });

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .with_state(state);

    let addr: SocketAddr = bind.parse()?;
    tracing::info!(%addr, coordinator = %coord, "openai-api listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn list_models(State(s): State<Arc<ApiState>>) -> Json<ModelList> {
    // Source of truth: mgmt registry when configured, otherwise a synthetic
    // "auto" placeholder so LM Studio (which requires a non-empty model list)
    // doesn't refuse to render the connection.
    if let Some(rows) = fetch_registry(&s).await {
        if !rows.is_empty() {
            let data = rows
                .into_iter()
                .filter(|m| {
                    m.get("status")
                        .and_then(|v| v.as_str())
                        .map(|s| s != "disabled")
                        .unwrap_or(true)
                })
                .map(|m| ModelEntry {
                    id: m.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    object: "model",
                    owned_by: "gpucluster".into(),
                })
                .collect();
            return Json(ModelList {
                object: "list",
                data,
            });
        }
    }
    let cluster_size = probe_cluster_size(&s).await;
    Json(ModelList {
        object: "list",
        data: vec![ModelEntry {
            id: format!("auto (cluster: {} nodes)", cluster_size),
            object: "model",
            owned_by: "gpucluster".into(),
        }],
    })
}

/// Per-request scratchpad — populated as we dispatch, flushed to the mgmt
/// inference log right before we return to the customer. One struct so the
/// caller can't accidentally write half the fields and forget the rest.
#[derive(Default)]
struct RequestTrace {
    request_id: Option<String>,
    api_key_prefix: Option<String>,
    model: String,
    node_id: Option<String>,
    inference_url: Option<String>,
    prompt_tokens: Option<i32>,
    completion_tokens: Option<i32>,
    total_tokens: Option<i32>,
    status_code: i32,
    error_type: Option<String>,
    error_message: Option<String>,
}

/// Top-level handler. Inspects the request body for `stream: true` and routes
/// to either the JSON dispatcher (full response, single body) or the streaming
/// dispatcher (SSE passthrough from llama-server).
async fn chat_completions(
    State(s): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<serde_json::Value>,
) -> Response {
    let wants_stream = req
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if wants_stream {
        return chat_completions_stream(s, headers, req).await;
    }
    match chat_completions_json(s, headers, req).await {
        Ok(j) => j.into_response(),
        Err((status, body)) => (status, body).into_response(),
    }
}

async fn chat_completions_json(
    s: Arc<ApiState>,
    headers: HeaderMap,
    req: serde_json::Value,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let started = Instant::now();
    let mut trace = RequestTrace::default();

    // Pick up identifiers from the request before we do anything else, so
    // even an early-failure log row carries the customer-visible context.
    trace.request_id = headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    trace.api_key_prefix = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(|tok| tok.chars().take(12).collect::<String>())
        .filter(|p| p.starts_with("mgc_"));
    trace.model = req
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("auto")
        .to_string();
    let req_messages_len = req
        .get("messages")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    // Pull the eligible-nodes view from the coordinator. This is also where
    // the dispatcher's first failure mode lives — if the coordinator is down
    // or returns garbage, we tag the trace and bail.
    let url = format!("{}/nodes/eligible", s.coordinator_url);
    let resp: serde_json::Value = match s.http.get(&url).send().await {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or(serde_json::Value::Null),
        Ok(r) => {
            return finalize_error(
                &s,
                trace,
                started,
                StatusCode::BAD_GATEWAY,
                "coordinator_error",
                format!("coordinator returned {}", r.status().as_u16()),
                serde_json::json!({
                    "error": { "type": "coordinator_error", "status": r.status().as_u16() }
                }),
            )
            .await;
        }
        Err(e) => {
            return finalize_error(
                &s,
                trace,
                started,
                StatusCode::BAD_GATEWAY,
                "coordinator_unreachable",
                e.to_string(),
                serde_json::json!({
                    "error": { "type": "coordinator_unreachable", "message": e.to_string() }
                }),
            )
            .await;
        }
    };

    let nodes = resp
        .get("nodes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Prefer a node that already has an inference endpoint (llama-server with
    // a model loaded). Falling back to "first eligible" lets us keep the 501
    // message informative when no worker has a model yet.
    let chosen = nodes
        .iter()
        .find(|n| {
            n.get("inference_url")
                .and_then(|v| v.as_str())
                .is_some()
        })
        .cloned()
        .or_else(|| nodes.first().cloned());

    let Some(chosen) = chosen else {
        let req_model_owned = trace.model.clone();
        return finalize_error(
            &s,
            trace,
            started,
            StatusCode::SERVICE_UNAVAILABLE,
            "no_eligible_nodes",
            "no worker is currently online with a usable GPU".into(),
            serde_json::json!({
                "error": {
                    "type": "no_eligible_nodes",
                    "message": "no worker is currently online with a usable GPU",
                    "request_model": req_model_owned,
                }
            }),
        )
        .await;
    };

    // Tag the trace with whatever the dispatcher chose, even before we know
    // the call will succeed — that way even a failed forward records who
    // got asked.
    trace.node_id = chosen
        .get("node_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    trace.inference_url = chosen
        .get("inference_url")
        .and_then(|v| v.as_str())
        .map(String::from);

    let Some(inference_url) = trace.inference_url.clone() else {
        let req_model_owned = trace.model.clone();
        return finalize_error(
            &s,
            trace,
            started,
            StatusCode::NOT_IMPLEMENTED,
            "no_inference_endpoint",
            "a worker is online but no model is loaded".into(),
            serde_json::json!({
                "error": {
                    "type": "no_inference_endpoint",
                    "message": "a worker is online but no model is loaded — load a model from the admin UI",
                    "phase": "2-dispatcher",
                    "request_model": req_model_owned,
                    "chosen_worker": chosen,
                }
            }),
        )
        .await;
    };

    // Forward the original chat-completion request to the chosen worker's
    // llama-server. This is the actual work of Phase 2: the worker has the
    // GGUF + GPU; we just relay JSON.
    forward_chat_to_worker(&s, &inference_url, &req, req_messages_len, trace, started).await
}

async fn forward_chat_to_worker(
    s: &ApiState,
    inference_url: &str,
    req: &serde_json::Value,
    req_messages_len: usize,
    mut trace: RequestTrace,
    started: Instant,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let upstream = format!("{}/v1/chat/completions", inference_url.trim_end_matches('/'));
    tracing::info!(
        upstream = %upstream,
        model = %trace.model,
        messages = req_messages_len,
        node = trace.node_id.as_deref().unwrap_or(""),
        request_id = trace.request_id.as_deref().unwrap_or(""),
        "forwarding chat completion to worker"
    );

    let resp = match s.http.post(&upstream).json(req).send().await {
        Ok(r) => r,
        Err(e) => {
            return finalize_error(
                s,
                trace,
                started,
                StatusCode::BAD_GATEWAY,
                "worker_unreachable",
                e.to_string(),
                serde_json::json!({
                    "error": {
                        "type": "worker_unreachable",
                        "upstream": upstream,
                        "message": e.to_string(),
                    }
                }),
            )
            .await;
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        // Map upstream status to a sensible client-facing one. Any 5xx
        // from llama-server bubbles up as 502 (the worker, not us, is
        // broken); 4xx passes through unchanged.
        let client_status = if status.is_server_error() {
            StatusCode::BAD_GATEWAY
        } else {
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY)
        };
        return finalize_error(
            s,
            trace,
            started,
            client_status,
            "worker_returned_error",
            format!("worker {} returned {}: {}", inference_url, status.as_u16(), body),
            serde_json::json!({
                "error": {
                    "type": "worker_returned_error",
                    "upstream_status": status.as_u16(),
                    "upstream_body": body,
                }
            }),
        )
        .await;
    }

    // llama-server speaks the OpenAI schema natively. We pass the response
    // through verbatim as `serde_json::Value` to preserve any extra fields
    // (`usage`, `system_fingerprint`, …). Streaming (`text/event-stream`) is
    // the next iteration; for now we assume the caller didn't set `stream`.
    let parsed = match resp.json::<serde_json::Value>().await {
        Ok(p) => p,
        Err(e) => {
            return finalize_error(
                s,
                trace,
                started,
                StatusCode::BAD_GATEWAY,
                "worker_response_parse_error",
                e.to_string(),
                serde_json::json!({
                    "error": {
                        "type": "worker_response_parse_error",
                        "message": e.to_string(),
                    }
                }),
            )
            .await;
        }
    };

    // Pull token counts out of llama-server's `usage` block. Three int
    // fields, all optional — older builds didn't always populate them.
    if let Some(usage) = parsed.get("usage") {
        trace.prompt_tokens = usage
            .get("prompt_tokens")
            .and_then(|v| v.as_i64())
            .map(|n| n as i32);
        trace.completion_tokens = usage
            .get("completion_tokens")
            .and_then(|v| v.as_i64())
            .map(|n| n as i32);
        trace.total_tokens = usage
            .get("total_tokens")
            .and_then(|v| v.as_i64())
            .map(|n| n as i32);
    }
    trace.status_code = 200;
    flush_log(s, &trace, started).await;
    Ok(Json(parsed))
}

/// SSE-streaming variant of `chat_completions_json`. Picks a worker the same
/// way, then proxies llama-server's `text/event-stream` response straight
/// through to the caller — no JSON parsing, no buffering. We still tag the
/// trace with the chosen node + status code; per-token usage isn't available
/// from a streaming response so prompt/completion counts stay null.
async fn chat_completions_stream(
    s: Arc<ApiState>,
    headers: HeaderMap,
    req: serde_json::Value,
) -> Response {
    let started = Instant::now();
    let mut trace = RequestTrace::default();

    trace.request_id = headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    trace.api_key_prefix = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(|tok| tok.chars().take(12).collect::<String>())
        .filter(|p| p.starts_with("mgc_"));
    trace.model = req
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("auto")
        .to_string();

    // 1) pick a worker via the coordinator
    let url = format!("{}/nodes/eligible", s.coordinator_url);
    let resp_json: serde_json::Value = match s.http.get(&url).send().await {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or(serde_json::Value::Null),
        Ok(r) => return stream_error(&s, trace, started,
            StatusCode::BAD_GATEWAY, "coordinator_error",
            format!("coordinator returned {}", r.status().as_u16())).await,
        Err(e) => return stream_error(&s, trace, started,
            StatusCode::BAD_GATEWAY, "coordinator_unreachable", e.to_string()).await,
    };

    let chosen = resp_json
        .get("nodes")
        .and_then(|v| v.as_array())
        .and_then(|a| a.iter().find(|n| n.get("inference_url").and_then(|v| v.as_str()).is_some()).cloned()
            .or_else(|| a.first().cloned()));

    let Some(chosen) = chosen else {
        return stream_error(&s, trace, started,
            StatusCode::SERVICE_UNAVAILABLE, "no_eligible_nodes",
            "no worker is currently online with a usable GPU".into()).await;
    };

    trace.node_id = chosen.get("node_id").and_then(|v| v.as_str()).map(String::from);
    trace.inference_url = chosen.get("inference_url").and_then(|v| v.as_str()).map(String::from);

    let Some(inference_url) = trace.inference_url.clone() else {
        return stream_error(&s, trace, started,
            StatusCode::NOT_IMPLEMENTED, "no_inference_endpoint",
            "a worker is online but no model is loaded".into()).await;
    };

    // 2) forward the request to the worker and pass its byte stream through
    let upstream = format!("{}/v1/chat/completions", inference_url.trim_end_matches('/'));
    tracing::info!(
        upstream = %upstream,
        model = %trace.model,
        node = trace.node_id.as_deref().unwrap_or(""),
        "streaming chat completion to worker"
    );

    let resp = match s.http.post(&upstream).json(&req).send().await {
        Ok(r) => r,
        Err(e) => return stream_error(&s, trace, started,
            StatusCode::BAD_GATEWAY, "worker_unreachable", e.to_string()).await,
    };

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let client_status = if status.is_server_error() {
            StatusCode::BAD_GATEWAY
        } else {
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY)
        };
        return stream_error(&s, trace, started, client_status,
            "worker_returned_error",
            format!("worker {} returned {}: {}", inference_url, status.as_u16(), body)).await;
    }

    // 3) Wrap reqwest's bytes_stream as an axum body and tag the response with
    // SSE headers. We don't try to parse events — llama-server already emits
    // valid `data: {...}\n\n` chunks. The customer's HTTP client (LM Studio,
    // OpenAI SDK, …) does the chunk-level parsing.
    trace.status_code = 200;
    flush_log(&s, &trace, started).await;

    let stream = resp.bytes_stream();
    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream; charset=utf-8")
        .header("cache-control", "no-cache, no-transform")
        .header("connection", "keep-alive")
        .header("x-accel-buffering", "no")  // disable nginx-style proxy buffering
        .body(body)
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "stream init failed").into_response())
}

async fn stream_error(
    s: &ApiState,
    mut trace: RequestTrace,
    started: Instant,
    status: StatusCode,
    error_type: &str,
    error_message: String,
) -> Response {
    trace.status_code = status.as_u16() as i32;
    trace.error_type = Some(error_type.to_string());
    trace.error_message = Some(error_message.clone());
    flush_log(s, &trace, started).await;
    let body = serde_json::json!({
        "error": { "type": error_type, "message": error_message }
    });
    (status, Json(body)).into_response()
}

/// Single exit point for the failure paths: tags the trace, fires off the log
/// write to mgmt, and returns the customer-facing error tuple. Keeps every
/// failure mode logged with consistent fields — there's no path out of
/// `chat_completions` that bypasses this.
async fn finalize_error(
    s: &ApiState,
    mut trace: RequestTrace,
    started: Instant,
    status: StatusCode,
    error_type: &str,
    error_message: String,
    body: serde_json::Value,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    trace.status_code = status.as_u16() as i32;
    trace.error_type = Some(error_type.to_string());
    trace.error_message = Some(error_message);
    flush_log(s, &trace, started).await;
    Err((status, Json(body)))
}

/// Fire-and-forget the log entry to mgmt-backend. We don't block the response
/// on this — if mgmt is down, the customer still gets their answer; the row
/// just won't appear in the admin UI. Logged at warn level so an operator
/// notices a sustained outage in the openai-api docker logs.
async fn flush_log(s: &ApiState, trace: &RequestTrace, started: Instant) {
    let Some(mgmt) = s.mgmt_url.as_deref() else {
        return;
    };
    let Some(token) = s.mgmt_token.as_deref() else {
        return;
    };
    let body = serde_json::json!({
        "request_id":        trace.request_id,
        "endpoint":          "chat.completions",
        "model":             trace.model,
        "node_id":           trace.node_id,
        "inference_url":     trace.inference_url,
        "api_key_prefix":    trace.api_key_prefix,
        "prompt_tokens":     trace.prompt_tokens,
        "completion_tokens": trace.completion_tokens,
        "total_tokens":      trace.total_tokens,
        "latency_ms":        started.elapsed().as_millis().min(i32::MAX as u128) as i32,
        "status_code":       trace.status_code,
        "error_type":        trace.error_type,
        "error_message":     trace.error_message,
    });
    let url = format!("{mgmt}/api/v1/inference/log");
    if let Err(e) = s
        .http
        .post(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
    {
        tracing::warn!(error = %e, "inference log write to mgmt failed");
    }
}

async fn probe_cluster_size(s: &ApiState) -> usize {
    let url = format!("{}/nodes", s.coordinator_url);
    match s.http.get(&url).send().await {
        Ok(r) if r.status().is_success() => {
            r.json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v.get("count").and_then(|n| n.as_u64()))
                .map(|n| n as usize)
                .unwrap_or(0)
        }
        _ => 0,
    }
}

/// Returns the model registry rows from mgmt-backend, or `None` if mgmt isn't
/// configured / unreachable. Caller falls back to the synthesised "auto" entry.
async fn fetch_registry(s: &ApiState) -> Option<Vec<serde_json::Value>> {
    let mgmt = s.mgmt_url.as_deref()?;
    let token = s.mgmt_token.as_deref()?;
    let url = format!("{mgmt}/api/v1/models");
    let res = s.http.get(url).bearer_auth(token).send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    res.json::<Vec<serde_json::Value>>().await.ok()
}
