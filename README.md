# MultiGPUCluster

> Distributed multi-GPU cluster for LLM inference and fine-tuning, with first-class support for heterogeneous hardware spread across the public internet.

[![CI](https://github.com/zurd46/MultiGPUCluster/actions/workflows/ci.yml/badge.svg)](https://github.com/zurd46/MultiGPUCluster/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Rust](https://img.shields.io/badge/rust-1.88-orange.svg)](https://www.rust-lang.org/)
[![CUDA](https://img.shields.io/badge/CUDA-12.8-green.svg)](https://developer.nvidia.com/cuda-toolkit)

MultiGPUCluster pools GPUs from machines at different locations into a single logical compute fabric. It speaks the OpenAI API so tools like **LM Studio** can use it as a drop-in inference backend, and runs distributed fine-tuning jobs across the same fleet.

The system is designed for **mixed hardware** — NVIDIA GPUs (RTX 5060 Ti + RTX 4090 + RTX 3090) **and Apple Silicon Macs (M1 → M4, Pro/Max/Ultra)** in the same cluster — and **public-internet deployment**: workers register over the internet through an encrypted WireGuard mesh and connect through a hardened gateway.

---

## Highlights

- **Heterogeneous-first scheduling** — VRAM-weighted layer placement, compute-group partitioning, mixed-precision aware (BF16 / FP8 / FP4).
- **Cross-vendor backends** — NVIDIA (CUDA / NCCL) **and Apple Silicon (Metal, unified memory)** in the same cluster. Mac workers run natively, Linux/Windows workers run in CUDA containers; the scheduler keeps them in separate compute groups for TP and stitches them together with pipeline parallelism for inference.
- **WAN-ready** — WireGuard overlay (Headscale) handles NAT traversal, dynamic IPs, and end-to-end encryption.
- **Auto-enrollment** — one-time token + mTLS cert issuance; workers reconnect automatically through reboots, ISP IP changes, and backend maintenance.
- **Zero-trust gateway** — TLS 1.3, mTLS for nodes, RBAC, rate limiting, immutable audit log, anomaly detection.
- **LM Studio compatible** — exposes `/v1/chat/completions` and `/v1/models` via an OpenAI-compatible layer.
- **Backend = system image, clients = containers or native binary** — a single `docker compose up` brings up the entire control plane; macOS workers ship as a native `.pkg`.
- **One URL for everything** — Caddy → Gateway fans out `/api/*` (mgmt), `/cluster/*` (coordinator), `/v1/*` (openai-api), `/enroll` (worker enrollment). A built-in **Cluster Management UI** on `/` aggregates all services live (KPI tiles, node-status donut, searchable tables, auto-refresh every 5 s); `/overview` returns the same data as JSON.
- **OpenAI-compatible URL with API keys** — `/v1/*` is bearer-protected. Customer keys (`mgc_*`) are minted in the admin UI, validated by the gateway against the mgmt-backend (Argon2id), and cached for 60 s to keep the verify hop off the hot path.
- **Live-editable backend config** — public base URL, default model, rate limits, **and the model registry that feeds `/v1/models`** are managed directly in the admin UI; node `pending_approval` / approve / drain / revoke actions ship in the same page.

---

## Architecture

```
                     Internet
                        │
                        ▼
┌───────────────────────────────────────────────────────────┐
│  BACKEND (system image, docker-compose / k8s)             │
│  ┌─────────────────────────────────────────────────────┐  │
│  │  Caddy (TLS, :443) → Edge Gateway (:8443)           │  │
│  │  ─ /  /overview   → admin UI + JSON aggregator      │  │
│  │  ─ /api/*         → mgmt-backend  (:7100)           │  │
│  │  ─ /cluster/*     → coordinator HTTP (:7001)        │  │
│  │  ─ /v1/*          → openai-api (:7200)              │  │
│  │  ─ /enroll        → mgmt-backend enrollment         │  │
│  └────────────────┬────────────────────────────────────┘  │
│  ┌────────────┐ ┌─┴──────────┐ ┌──────────────┐           │
│  │Coordinator │ │Mgmt Backend│ │OpenAI API    │           │
│  │ HTTP :7001 │ │   :7100    │ │   :7200      │           │
│  │ gRPC :7000 │ │+ RBAC/Audit│ │(LM Studio)   │           │
│  │+ Scheduler │ │            │ │              │           │
│  └─────┬──────┘ └─────┬──────┘ └──────┬───────┘           │
│  ┌─────┴───────────────┴───────────────┴─────┐            │
│  │  PostgreSQL · Redis · MinIO · Headscale   │            │
│  └───────────────────────────────────────────┘            │
└───────────────────────────┬───────────────────────────────┘
                            │ WireGuard mesh
       ┌────────────────────┼──────────────────────┬────────────────┐
       ▼                    ▼                      ▼                ▼
   Site A: Win11+WSL2    Site B: Linux         Site C: Linux    Site D: macOS
   ┌─────────────┐       ┌──────────┐          ┌──────────┐    ┌──────────────┐
   │ Bootstrapper│       │Bootstrap.│          │Bootstrap.│    │ Bootstrapper │
   │      ↓      │       │    ↓     │          │    ↓     │    │      ↓       │
   │Worker (CUDA)│       │Worker    │          │Worker    │    │Worker (Metal)│
   │  RTX 5060Ti │       │ 2× 4090  │          │ RTX 3090 │    │  M3 Max 64GB │
   └─────────────┘       └──────────┘          └──────────┘    └──────────────┘
```

See [`docs/PLAN.md`](docs/PLAN.md) for the full architecture document, including scheduler internals, security model, and phase-by-phase roadmap.

---

## Features

### Inference
- OpenAI-compatible HTTP API (`/v1/chat/completions`, `/v1/models`)
- Distributed inference via an extended fork of `llama.cpp`'s RPC backend (`rpc-server-ext`, builds with either ggml-cuda or ggml-metal)
- Pipeline parallelism across WAN, tensor parallelism within latency islands
- Heterogeneous GPUs handled natively (5060 Ti + 4090 + 3090 + Apple M3 Max → one model)

### Fine-tuning
- LoRA / QLoRA over `candle` (Rust) — no Python required for the common path
- DDP + FSDP via NCCL for larger jobs (NVIDIA-only — Metal lacks a cross-node collective today; Apple workers stay inference-only until Phase 6)
- Geo-aware data placement (datasets can be pinned to specific regions)

### Cluster management
- Auto-enrollment with one-time tokens, short-lived mTLS certs (7-day TTL), auto-renewal
- Per-node identity: UUIDv7 ID + hardware fingerprint, persistent across reboots
- **Full inventory visible from every client** — device name (`scutil --get ComputerName` on macOS, `hostnamectl --pretty` on Linux, hostname elsewhere), OS family + version + kernel + arch, CPU model + cores, RAM, every GPU with backend / architecture / VRAM / cores / precision support. The same block is printed by `gpucluster-worker` at startup, by `gpucluster-agent status`, and during enrollment so the operator confirms what's about to be uploaded.
- **All inventory streams to the gateway** — workers POST a full `NodeInfo` snapshot to `/cluster/nodes/report` every 30 s (gateway proxies to coordinator). Same JSON shape on enrollment and heartbeat, so registration and updates share one code path. `GET /cluster/nodes` returns the full inventory back, decorated with `last_heartbeat` and the TLS-socket-captured `current_public_ip`.
- WAN-IP history per node (authoritative, captured at TLS socket level)
- Driver-mismatch quarantine (NCCL incompatibility detection)
- Status lifecycle: `pending_approval` → `online` → `degraded`/`draining`/`offline`/`quarantined`/`revoked`

### Security
- TLS 1.3 only, modern cipher suites
- mTLS for all worker ↔ backend traffic, internal service-to-service mTLS
- Argon2id-hashed API keys, OAuth2/OIDC for the web UI, optional 2FA (TOTP)
- Sliding-window rate limiting (per IP / per token / per user)
- Immutable audit log with optional SIEM export
- Hardware-fingerprint binding, optional TPM key storage

---

## Quick Start

### Prerequisites
- **Backend host:** Linux VPS or cloud VM, Docker + Docker Compose, public domain with DNS pointing to it.
- **NVIDIA worker host:** Linux (native) or Windows 11 with WSL2 + Docker Desktop, NVIDIA driver ≥ 535 (≥ 555 for Blackwell / RTX 50-series), NVIDIA Container Toolkit.
- **Apple Silicon worker host:** macOS 14 (Sonoma) or newer on M1/M2/M3/M4 (any variant). No Docker required — the worker runs as a native `launchd` daemon. Intel Macs are not supported.

### 1. Deploy the backend

```bash
git clone https://github.com/zurd46/MultiGPUCluster.git
cd MultiGPUCluster/backend

cp .env.example .env
# Fill in POSTGRES_PASSWORD, JWT_SECRET, MINIO_ROOT_PASSWORD,
# BACKEND_DOMAIN, and ADMIN_API_KEY (used for the admin UI / mgmt admin endpoints).

docker compose up -d --build
```

Caddy provisions a TLS cert from Let's Encrypt for `BACKEND_DOMAIN`. Once up, **everything is reachable under one URL**:

```bash
# Production (Caddy → Gateway, TLS terminated by Caddy)
open https://cluster.example.com/        # Cluster Management UI
curl https://cluster.example.com/health  # {"status":"ok"}
curl https://cluster.example.com/overview  # aggregated JSON across all services

# Local dev (Gateway directly, no TLS)
open http://localhost:8443/
curl http://localhost:8443/overview
```

The admin page on `/` shows live service health, the coordinator's node registry, the mgmt-backend's enrolled nodes, and the OpenAI-API model list — auto-refreshes every 5 s. To see admin-protected mgmt data (`/api/v1/nodes`), paste your `ADMIN_API_KEY` into the field at the top of the page.

#### Routes exposed under the one URL

| Path | Upstream | Purpose | Auth |
|---|---|---|---|
| `/` | gateway | Cluster Management UI | none (admin key entered in UI) |
| `/overview` | gateway | JSON aggregator (services + nodes + models) | optional admin |
| `/health`, `/ready` | gateway | Liveness / readiness | none |
| `/api/v1/nodes`, `/api/v1/audit`, `/api/v1/enroll/tokens` | mgmt | Node + audit + enrollment-token admin | `Authorization: Bearer ADMIN_API_KEY` |
| `/api/v1/keys`, `/api/v1/keys/{id}`, `/api/v1/keys/{id}/revoke`, `/api/v1/keys/verify` | mgmt | Customer-key CRUD + service-side verify | admin |
| `/api/v1/settings` (GET/PUT) | mgmt | Cluster-wide settings KV (public base URL, default model, rate limit, max tokens) | admin |
| `/api/v1/models` (GET/POST) · `/api/v1/models/{id}` (PATCH/DELETE) | mgmt | Model registry powering `/v1/models` | admin |
| `/cluster/nodes`, `/cluster/nodes/eligible`, `/cluster/nodes/report` | coordinator (`:7001`) | Live registry + dispatch view + worker upload | mTLS (workers) / none in dev |
| `/v1/models`, `/v1/chat/completions` | openai-api (`:7200`) | OpenAI-compatible | `Authorization: Bearer mgc_<token>` (customer key) |
| `/enroll` | mgmt | Worker enrollment (alias for `/api/v1/enroll`) | one-time enrollment token |

### 2. Generate an enrollment token

```bash
gpucluster nodes token --display "workstation-dani"
# → eyJhbGciOiJ...   (one-time, 15-minute TTL)
```

(Or use the web dashboard once it ships in Phase 5.)

### 3. Install the agent on a worker host

**Linux:**
```bash
curl -fsSL https://cluster.example.com/install.sh | sudo bash
sudo gpucluster-agent enroll \
  --backend https://cluster.example.com \
  --token   <ONE_TIME_TOKEN> \
  --display-name "ai-rig-01"
```

**Windows 11 (WSL2 + Docker Desktop):**
```powershell
# Ensure WSL2 mirrored networking (one-time):
# %USERPROFILE%\.wslconfig
# [wsl2]
# networkingMode=mirrored

iwr https://cluster.example.com/install.ps1 | iex
gpucluster-agent enroll `
  --backend https://cluster.example.com `
  --token   <ONE_TIME_TOKEN> `
  --display-name "workstation-dani"
```

**macOS (Apple Silicon):**
```bash
# .pkg installs gpucluster-agent + gpucluster-worker + rpc-server-ext into /usr/local/bin
curl -fsSL https://cluster.example.com/install-macos.sh | sudo bash
sudo gpucluster-agent enroll \
  --backend https://cluster.example.com \
  --token   <ONE_TIME_TOKEN> \
  --display-name "macbook-dani"
```

The agent installs itself as a systemd unit (Linux) / Windows Service / `launchd` daemon (macOS), enrolls, fetches its mTLS cert, joins the WireGuard mesh, and starts the appropriate worker — a CUDA container on Linux/WSL2, or the native Metal worker on macOS. From now on it auto-reconnects on every boot.

### 4. Mint a customer API key

`/v1/*` is bearer-protected — every request needs an `Authorization: Bearer mgc_…` header. Keys are minted in the admin UI under "API keys", or directly via the API:

```bash
curl -X POST -H "Authorization: Bearer $ADMIN_API_KEY" \
     -H "Content-Type: application/json" \
     -d '{"name":"lm-studio-laptop","scope":"inference"}' \
     https://cluster.example.com/api/v1/keys
# → { "id": "...", "token": "mgc_<32 base64 chars>", "prefix": "mgc_<8>", ... }
```

The `token` is shown **only once**. The admin UI also lets you edit, revoke (soft, audit-preserving), or hard-delete keys, and keeps a `last_used_at` timestamp for each.

### 5. Register a model + cluster settings (one-time)

The admin UI has two new panels:

- **Cluster settings** — public base URL, default model, rate limit (rpm), max tokens default. Stored in the `cluster_settings` table; `/api/v1/settings` backs the UI.
- **Models** — the registry that feeds `/v1/models`. Add an entry per model the cluster serves; mark one as default. Phase 2 syncs `status` from live worker state.

```bash
# Register a model
curl -X POST -H "Authorization: Bearer $ADMIN_API_KEY" \
     -H "Content-Type: application/json" \
     -d '{"id":"llama-3.1-8b-instruct","display_name":"Llama 3.1 8B Instruct","is_default":true}' \
     https://cluster.example.com/api/v1/models
```

### 6. Use it from LM Studio

Point LM Studio (or any OpenAI-compatible client) at:

```
Base URL:  https://cluster.example.com/v1
API key:   mgc_<token from step 4>
```

---

## Project Structure

```
MultiGPUCluster/
├── crates/                              Rust workspace members
│   ├── proto/                           gRPC + protobuf definitions
│   ├── common/                          Shared types, errors, IDs
│   ├── sysinfo/                         NVML + OS detection (Win/Linux)
│   ├── ca/                              Internal certificate authority
│   ├── gateway/                         One-URL fan-out + admin UI + mTLS/WAF/audit
│   ├── coordinator/                     Cluster master + scheduler glue
│   ├── scheduler/                       Placement algorithms, compute groups
│   ├── mgmt-backend/                    Users, RBAC, audit, enrollment
│   ├── openai-api/                      LM Studio compat layer
│   ├── worker/                          Node agent (runs in container)
│   └── nccl-wrapper/                    NCCL FFI (feature-gated)
├── bootstrapper/                        Native host agent (systemd / WinSvc)
├── cli/                                 `gpucluster` admin CLI
├── cpp/
│   ├── llama-rpc-ext/                   llama.cpp fork with cluster hooks
│   └── cuda-kernels/                    Custom kernels (fine-tuning)
├── dashboard/                           Standalone web UI (Phase 5 — minimal admin already shipped inside gateway)
├── backend/
│   ├── docker-compose.yml               Backend stack
│   ├── Caddyfile                        Reverse proxy + TLS
│   └── helm/                            K8s chart (Phase 5)
├── docker/                              Per-service Dockerfiles
├── docs/
│   └── PLAN.md                          Full architecture document
└── .github/workflows/                   CI
```

---

## Tech Stack

| Layer | Choice |
|---|---|
| Control plane | Rust (tokio, tonic, axum, sqlx) |
| Compute (NVIDIA) | C++ / CUDA, NCCL, llama.cpp fork (ggml-cuda) |
| Compute (Apple Silicon) | C++ / Metal, llama.cpp fork (ggml-metal), unified memory |
| Fine-tuning | candle (Rust) — PyTorch via pyo3 as fallback (NVIDIA only today) |
| Worker packaging | Docker image (Linux/WSL2) · native `.pkg` + `launchd` (macOS) |
| Database | PostgreSQL 16 |
| Cache / queue | Redis 7 |
| Object storage | MinIO (S3-compatible) |
| Reverse proxy / TLS | Caddy 2 |
| Mesh VPN | Headscale (WireGuard) |
| Observability | OpenTelemetry, Prometheus, Grafana |

**Why Rust:** Memory safety and no GC pauses for the scheduler / gateway hot path.
**Why C++:** CUDA kernels, NCCL, Metal shaders, and llama.cpp are natively C++.

---

## Roadmap

| Phase | Focus | Status |
|---|---|---|
| 0 | Foundation: workspace, Dockerfiles, bootstrapper skeleton, **gateway reverse-proxy + built-in admin UI on a single URL** | done |
| 1 | Identity & cluster fundamentals: enrollment, mTLS, registry, WAN-IP tracking | in progress |
| 2 | Distributed inference over WAN: llama.cpp RPC fork, layer-allocation solver, OpenAI API | planned |
| 3 | Smart scheduling & observability: priority queues, NCCL bench, Prometheus | planned |
| 4 | Fine-tuning: LoRA / QLoRA / FSDP, dataset registry with privacy tags | planned |
| 5 | Production UI & hardening: web dashboard, Helm chart, pen-test, DR runbook | planned |

Detailed milestones live in [`docs/PLAN.md`](docs/PLAN.md).

---

## Heterogeneous Cluster Notes

Mixed architectures are a **first-class** scenario, not an edge case. The scheduler:

1. Builds a capability profile per GPU at join time (architecture, backend, FP8/FP4 support, measured TFLOPs).
2. Partitions the cluster into homogeneous **compute groups** for tensor parallelism — bucket key is `cuda-{arch}-{cc.major}.{cc.minor}` for NVIDIA, `metal-{family}` for Apple (e.g. `metal-Apple-M3-Max`). NVIDIA and Apple GPUs are never lumped into the same TP group.
3. Runs **pipeline parallelism** across groups, with VRAM- and benchmark-score-weighted layer allocation. Mixed CUDA/Metal pipelines are allowed at PP boundaries because both backends agree on BF16 on the wire.
4. Detects latency islands by RTT measurement — TP only inside an island, PP across the WAN.
5. Picks cluster-wide precision as the greatest common denominator (BF16 default, FP8 when all selected GPUs support it). Apple GPUs older than M3 (no native BF16) and any Apple GPU when FP8/FP4 is required are filtered out automatically.

**Example:** A 70B model split across `2× RTX 4090 + 1× RTX 5060 Ti + 1× RTX 3090 + 1× M3 Max`:
```
Stage 0:  RTX 5060 Ti  →  ~10 layers (cuda-Blackwell-12.0)
Stage 1:  RTX 4090 #1  →  ~22 layers (cuda-Ada-8.9)
Stage 2:  RTX 4090 #2  →  ~22 layers (cuda-Ada-8.9)
Stage 3:  RTX 3090     →  ~14 layers (cuda-Ampere-8.6)
Stage 4:  M3 Max 64GB  →  ~12 layers (metal-Apple-M3-Max)
```

---

## Security Model

The gateway is the only component exposed to the public internet. Everything else lives behind it on a private Docker network.

| Layer | Mechanism |
|---|---|
| L1 — TCP | SYN cookies, per-IP connection limits, fail2ban-style auto-ban |
| L2 — TLS | TLS 1.3, HSTS, OCSP stapling, cert pinning for clients |
| L3 — Auth | mTLS (workers) · API keys (Argon2id) · OAuth2/OIDC (UI) |
| L4 — RBAC | `admin` / `operator` / `user` / `viewer` + per-resource permissions + quotas |
| L5 — Rate limit | Sliding window per IP/token/user, adaptive on anomaly |
| L6 — WAF | Strict schema validation, body/header limits, optional prompt-injection heuristics |
| L7 — Audit | Append-only Postgres log, optional SIEM export |

All inter-service traffic inside the backend is mTLS-only. Worker certs have a 7-day TTL with auto-renewal, so a stolen key is wirkungslos within a week.

---

## Development

```bash
# Workspace build
cargo build --workspace --release

# Lint + format
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings

# Run a single component locally
cargo run -p gpucluster-gateway
cargo run -p gpucluster-coordinator
cargo run -p gpucluster-mgmt-backend

# Worker — auto-detects backend from the host:
#   Linux/Windows + NVIDIA driver  → CUDA RPC backend
#   macOS on Apple Silicon         → Metal RPC backend
#   anything else                  → enrolls but stays inference-ineligible
cargo run -p gpucluster-worker
```

For the C++ RPC server pick the matching backend explicitly:

```bash
# NVIDIA host
cmake -S cpp/llama-rpc-ext -B cpp/llama-rpc-ext/build \
  -DBUILD_RPC_SERVER=ON -DBUILD_BACKEND_CUDA=ON
cmake --build cpp/llama-rpc-ext/build -j

# Apple Silicon host (default on macOS)
cmake -S cpp/llama-rpc-ext -B cpp/llama-rpc-ext/build \
  -DBUILD_RPC_SERVER=ON -DBUILD_BACKEND_METAL=ON
cmake --build cpp/llama-rpc-ext/build -j
```

### Bring up the full backend stack locally

Two options — pick whichever is convenient:

**A) Docker Compose (recommended — also brings up Postgres / Redis / MinIO / Caddy):**

```bash
cd backend
docker compose up -d --build
open http://localhost:8443/        # Cluster Management UI (gateway direct)
open http://localhost/             # via Caddy (uses BACKEND_DOMAIN, default localhost)
```

**B) Native dev (no Docker — needs Postgres running on :5432):**

```bash
./scripts/dev-up.sh
# health checks printed at the end. Stop with: ./scripts/dev-down.sh
```

Local default ports:

| Service | Port |
|---|---|
| gateway (HTTP, admin UI, all routes) | `8443` |
| coordinator HTTP | `7001` |
| coordinator gRPC | `7000` |
| mgmt-backend | `7100` |
| openai-api | `7200` |

The `crates/proto` package compiles `.proto` files via `tonic-build` at build time. Make sure `protoc` is installed locally (`apt install protobuf-compiler` / `brew install protobuf`).

### Environment variables (backend)

| Var | Used by | Purpose |
|---|---|---|
| `BACKEND_DOMAIN` | caddy | Public domain for TLS / routing (default `localhost` for dev) |
| `POSTGRES_PASSWORD` | postgres, mgmt | DB password |
| `JWT_SECRET` | mgmt | JWT signing key |
| `ADMIN_API_KEY` | mgmt, gateway admin UI | Bearer token for `/api/v1/...` admin endpoints |
| `MINIO_ROOT_USER` / `MINIO_ROOT_PASSWORD` | minio | Object storage credentials |
| `GATEWAY_BIND` | gateway | Gateway bind address (default `0.0.0.0:8443`) |
| `MGMT_BACKEND_URL` | gateway | Upstream URL for `/api/*` (default `http://mgmt:7100`) |
| `COORDINATOR_HTTP_URL` | gateway | Upstream URL for `/cluster/*` (default `http://coordinator:7001` — note: HTTP port, not gRPC) |
| `OPENAI_API_URL` | gateway | Upstream URL for `/v1/*` (default `http://openai-api:7200`) |
| `MGMT_BACKEND_URL` | openai-api | Upstream URL of mgmt-backend, used to source `/v1/models` from the live registry |
| `COORDINATOR_HTTP_URL` | openai-api | Coordinator HTTP base (`:7001`) for `/v1/chat/completions` dispatch — **not** the gRPC port |

---

## Documentation

- [`docs/PLAN.md`](docs/PLAN.md) — full architecture, sub-system specs, phase-by-phase roadmap (German + English mixed)

More docs land alongside each phase milestone.

---

## Contributing

Contributions are welcome once the project hits Phase 1. Until then the API surface and on-disk layouts are still in flux.

If you have feedback on the architecture in `docs/PLAN.md`, open a discussion or issue.

---

## License

Apache License 2.0 — see [`LICENSE`](LICENSE) (added with first public release).

---

## Acknowledgments

- [`llama.cpp`](https://github.com/ggerganov/llama.cpp) — foundation for the RPC backend
- [`Headscale`](https://github.com/juanfont/headscale) — open-source Tailscale coordinator powering the WireGuard mesh
- NVIDIA NCCL, CUDA, and the broader open ML stack
