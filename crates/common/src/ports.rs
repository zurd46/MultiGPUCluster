//! Single source of truth for the cluster's port assignments.
//!
//! Why a const module instead of a config file: these ports are part of the
//! cluster's *protocol*. Workers and the coordinator must agree on them
//! before any config is exchanged (a worker dials the coordinator on its
//! own opinion of what the port is, before any /settings API exists). So
//! the ports live in code, in one place, and every binary inherits the
//! same view.
//!
//! What is *not* here: per-binary `--bind` defaults. Those are user-tunable
//! at deploy time and live in the binary's `clap` Args (e.g. `OPENAI_API_BIND`
//! still defaults to `0.0.0.0:OPENAI_API`). The constants here are the
//! contract between two services about which port a peer is listening on.

/// Coordinator's gRPC listener — used by `tonic` clients (Phase 3+).
pub const COORDINATOR_GRPC: u16 = 7000;

/// Coordinator's HTTP listener — handles `/nodes`, `/nodes/report`, the
/// `/nodes/{id}/load_model` proxy. Internal-only; the gateway proxies
/// `/cluster/*` to here.
pub const COORDINATOR_HTTP: u16 = 7001;

/// mgmt-backend HTTP listener — admin API (`/api/v1/*`).
pub const MGMT_BACKEND_HTTP: u16 = 7100;

/// openai-api HTTP listener — customer-facing OpenAI-compatible endpoints.
pub const OPENAI_API_HTTP: u16 = 7200;

/// Public-facing gateway. Caddy terminates TLS in front of this; the
/// gateway itself speaks plaintext HTTP/2 inside the backend network.
pub const GATEWAY_HTTP: u16 = 8443;

/// llama.cpp `rpc-server-ext` (the GGML RPC backend) on every worker.
/// Used by the dispatcher when it does multi-node tensor-parallel.
pub const WORKER_RPC: u16 = 50052;

/// llama.cpp `llama-server` (the OpenAI-compatible HTTP layer) on every
/// worker. The dispatcher hits this for single-node inference.
pub const WORKER_INFERENCE: u16 = 50053;

/// Worker-local control plane — receives admin-initiated commands like
/// `load_model` from the coordinator's proxy.
pub const WORKER_CONTROL: u16 = 50054;
