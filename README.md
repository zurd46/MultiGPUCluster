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

---

## Architecture

```
                     Internet
                        │
                        ▼
┌───────────────────────────────────────────────────────────┐
│  BACKEND (system image, docker-compose / k8s)             │
│  ┌─────────────────────────────────────────────────────┐  │
│  │  Caddy (TLS) → Edge Gateway (mTLS, WAF, RateLimit)  │  │
│  └────────────────┬────────────────────────────────────┘  │
│  ┌────────────┐ ┌─┴──────────┐ ┌──────────────┐           │
│  │Coordinator │ │Mgmt Backend│ │OpenAI API    │           │
│  │+ Scheduler │ │+ RBAC/Audit│ │(LM Studio)   │           │
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
- Full inventory: OS, GPU model + architecture, driver version, CUDA version, VBIOS, public IP, ASN, geo
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
# and BACKEND_DOMAIN.

docker compose up -d --build
```

Caddy provisions a TLS cert from Let's Encrypt for `BACKEND_DOMAIN`. Once up:

```bash
curl https://cluster.example.com/health
# {"status":"ok"}
```

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

The agent installs itself as a systemd unit / Windows Service, enrolls, fetches its mTLS cert, joins the WireGuard mesh, pulls the matching CUDA-tagged worker container, and starts. From now on it auto-reconnects on every boot.

### 4. Use it from LM Studio

Point LM Studio (or any OpenAI-compatible client) at:

```
Base URL:  https://cluster.example.com/v1
API key:   <your_api_key>
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
│   ├── gateway/                         Edge gateway (mTLS, WAF, audit)
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
├── dashboard/                           Web UI (Phase 5)
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
| Compute | C++ / CUDA, NCCL, llama.cpp fork |
| Fine-tuning | candle (Rust) — PyTorch via pyo3 as fallback |
| Database | PostgreSQL 16 |
| Cache / queue | Redis 7 |
| Object storage | MinIO (S3-compatible) |
| Reverse proxy / TLS | Caddy 2 |
| Mesh VPN | Headscale (WireGuard) |
| Observability | OpenTelemetry, Prometheus, Grafana |

**Why Rust:** Memory safety and no GC pauses for the scheduler / gateway hot path.
**Why C++:** CUDA kernels, NCCL, and llama.cpp are natively C++.

---

## Roadmap

| Phase | Focus | Status |
|---|---|---|
| 0 | Foundation: workspace, Dockerfiles, bootstrapper skeleton | scaffolded |
| 1 | Identity & cluster fundamentals: enrollment, mTLS, registry, WAN-IP tracking | planned |
| 2 | Distributed inference over WAN: llama.cpp RPC fork, layer-allocation solver, OpenAI API | planned |
| 3 | Smart scheduling & observability: priority queues, NCCL bench, Prometheus | planned |
| 4 | Fine-tuning: LoRA / QLoRA / FSDP, dataset registry with privacy tags | planned |
| 5 | Production UI & hardening: web dashboard, Helm chart, pen-test, DR runbook | planned |

Detailed milestones live in [`docs/PLAN.md`](docs/PLAN.md).

---

## Heterogeneous Cluster Notes

Mixed architectures are a **first-class** scenario, not an edge case. The scheduler:

1. Builds a capability profile per GPU at join time (architecture, FP8/FP4 support, measured TFLOPs).
2. Partitions the cluster into homogeneous **compute groups** for tensor parallelism.
3. Runs **pipeline parallelism** across groups, with VRAM- and benchmark-score-weighted layer allocation.
4. Detects latency islands by RTT measurement — TP only inside an island, PP across the WAN.
5. Picks cluster-wide precision as the greatest common denominator (BF16 default, FP8 when all selected GPUs support it).

**Example:** A 70B model split across `2× RTX 4090 + 1× RTX 5060 Ti + 1× RTX 3090`:
```
Stage 0:  RTX 5060 Ti  →  ~12 layers
Stage 1:  RTX 4090 #1  →  ~26 layers
Stage 2:  RTX 4090 #2  →  ~26 layers
Stage 3:  RTX 3090     →  ~16 layers
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

# Worker (requires NVIDIA driver + CUDA)
cargo run -p gpucluster-worker
```

The `crates/proto` package compiles `.proto` files via `tonic-build` at build time. Make sure `protoc` is installed locally (`apt install protobuf-compiler` / `brew install protobuf`).

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
