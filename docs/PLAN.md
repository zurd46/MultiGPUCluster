# MultiGPUCluster — Projekt-Plan

Verteiltes Multi-GPU-Cluster-System für Inferenz und Fine-Tuning mit LM Studio-Kompatibilität.
Heterogene Clients (Windows/Linux mit NVIDIA + macOS mit Apple Silicon) **an verschiedenen Standorten** werden über das Internet zu einem logischen GPU-Pool zusammengeführt und intelligent für Workloads verteilt.

**Kern-Eigenschaften:**
- Backend zentral als **System-Image** (komplette Verwaltung, Gateway, Coordinator)
- Clients als schlanke **Container** auf User-Hosts (Win via WSL2 / Linux nativ)
- **Auto-Enrollment über Internet** mit Secure-Onboarding
- **Zero-Trust-Gateway** mit mTLS, Audit, Rate-Limiting
- **WAN-tauglich** via Mesh-VPN (WireGuard-Overlay)

---

## 1. Architektur-Übersicht

```
                    Internet
                       │
                       ▼
┌──────────────────────────────────────────────────────────────┐
│  BACKEND-SYSTEM (System-Image / Cloud-Deploy)                │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │  Edge Gateway (TLS 1.3, mTLS, WAF, Rate-Limit, Audit)   │ │
│  └────────────────────┬────────────────────────────────────┘ │
│                       │                                      │
│  ┌──────────────┐ ┌───┴────────────┐ ┌──────────────────┐    │
│  │ Coordinator  │ │ Mgmt-Backend   │ │ OpenAI-API-Layer │    │
│  │ (Scheduler,  │ │ (Users, RBAC,  │ │ (LM Studio       │    │
│  │  Job-Queue)  │ │  Audit, Quotas)│ │  Compat)         │    │
│  └──────┬───────┘ └────────┬───────┘ └────────┬─────────┘    │
│         │                  │                  │              │
│  ┌──────┴──────────────────┴──────────────────┴────────┐     │
│  │  PostgreSQL  ·  Redis  ·  Object-Store (Models)     │     │
│  └─────────────────────────────────────────────────────┘     │
│  ┌─────────────────────────────────────────────────────┐     │
│  │  WireGuard-Mesh-Hub (Headscale/Custom Tailnet)      │     │
│  └─────────────────────────────────────────────────────┘     │
└──────────────────────────┬───────────────────────────────────┘
                           │  WireGuard Overlay (verschlüsselt, NAT-Traversal)
       ┌───────────────────┼─────────────────────┬──────────────┐
       ▼                   ▼                     ▼              ▼
   Standort A          Standort B           Standort C    ...
   ┌─────────┐         ┌─────────┐          ┌─────────┐
   │Worker   │         │Worker   │          │Worker   │
   │Container│         │Container│          │Container│
   │Win+WSL2 │         │ Linux   │          │ Linux   │
   │RTX5060Ti│         │ 2×4090  │          │ RTX3090 │
   └─────────┘         └─────────┘          └─────────┘
```

---

## 2. Komponenten & Tech-Stack

### Backend (System-Image, läuft zentral)

| Komponente | Sprache | Zweck |
|---|---|---|
| **Edge Gateway** | Rust (axum + tower) | TLS-Terminierung, mTLS, Rate-Limit, WAF, Audit, DDoS-Schutz |
| **Coordinator** | Rust (tokio, tonic) | Cluster-State, Scheduling, Job-Queue |
| **Management-Backend** | Rust (axum) + DB | User-Mgmt, RBAC, Audit, Quotas, Node-Approval |
| **OpenAI-API-Layer** | Rust (axum) | OpenAI-kompatibler Endpoint für LM Studio & Apps |
| **WG-Hub** | Headscale (Go) o. Custom | WireGuard-Mesh-Coordinator, NAT-Traversal |
| **Datastores** | PostgreSQL, Redis, S3/MinIO | Metadaten, Cache, Model-/Checkpoint-Storage |
| **Web-UI (Admin)** | SvelteKit / Next.js | Cluster-Verwaltung, Monitoring |

### Clients (Container *oder* native Binary, laufen bei den Usern)

| Komponente | Sprache | Zweck |
|---|---|---|
| **Worker Agent** | Rust + NVML / Metal-Discovery | GPU-Discovery (CUDA + Apple Silicon), Heartbeat, Job-Execution |
| **Inference-Backend** | C++ (llama.cpp Fork) | Verteilte Inferenz; ggml-cuda *oder* ggml-metal je nach Host |
| **Fine-Tuning-Backend** | C++/CUDA + Rust (candle) | LoRA/QLoRA, FSDP, DDP — **CUDA-only** (Metal kommt in Phase 6) |
| **NCCL-Wrapper** | C++ + Rust FFI | Tensor-Sync zwischen NVIDIA-GPUs |
| **WireGuard-Client** | wireguard-go / kernel | VPN-Tunnel zum Backend |
| **Bootstrapper** | Rust (native Binary) | Host-Setup, Container- *oder* Native-Worker-Lifecycle, Enrollment |

**Cross-vendor Inventory:** Jede `GpuInfo`-Message trägt ein `GpuBackend`-Enum (`CUDA`/`METAL`/`ROCM`/`VULKAN`) plus `unified_memory`-Flag und `gpu_core_count`. Apple Silicon meldet `architecture = "Apple-M3-Max"` o.ä. statt einer Compute-Capability — der Scheduler bucketet entsprechend (`metal-Apple-M3-Max` vs `cuda-Ampere-8.0`).

**Warum Rust:** Memory-Safety + Performance, ideal für Gateway/Coordinator (kein GC-Stutter).
**Warum C++:** CUDA-Kernels, NCCL, llama.cpp-Integration sind nativ C++.

---

## 3. Deployment-Modell: Image vs Container

### Backend = System-Image
Das Backend ist eine **komplette deploybare Einheit** — z.B.:
- **Docker-Compose-Stack** (für Selbst-Hosting, einfachster Weg)
- **Kubernetes-Helm-Chart** (für Production/HA)
- **VM-Image** (Packer-built, AWS AMI / Proxmox / Hetzner)

Inhalt: Gateway + Coordinator + Mgmt-Backend + Postgres + Redis + MinIO + WG-Hub + Reverse-Proxy. Eine Installation, ein Update-Pfad.

### Clients = Container ODER Native Binary

**Linux / Windows+WSL2 (NVIDIA):** Schlankes Worker-Image (`gpucluster/worker:VERSION-cudaXX.X`), das vom Bootstrapper auf User-Hosts gestartet wird. Keine separate Installation, kein OS-Eingriff jenseits Docker/WSL2.

**macOS (Apple Silicon):** Native `.pkg`-Distribution mit `gpucluster-agent` + `gpucluster-worker` + `rpc-server-ext` als universal-Binaries unter `/usr/local/bin/`. Kein Docker — Metal-Devices können nicht in Linux-Container durchgereicht werden, der Worker läuft daher als launchd-Daemon (`com.gpucluster.agent`) direkt auf dem Host. Selbe Enrollment-Story, selbe Mesh-VPN, selber Coordinator-Endpoint.

```
Verteilung pro Plattform:
  Linux/WSL2  → Docker-Image (FROM nvidia/cuda)        → CUDA RPC
  macOS AS    → /Library/Application Support/gpucluster → Metal RPC (launchd)
```

---

## 4. Netzwerk-Topologie: WAN über Mesh-VPN

### Problem

Nodes sind an verschiedenen Standorten (Heim-Internet, Cloud-VPS, Office-Netzwerke) → unterschiedliche NAT-Konfigurationen, dynamische IPs, public Internet als Transport.

NCCL erwartet aber: niedrige Latenz, hohe Bandbreite, direktes Routing. Das geht **nicht** über offenes Internet.

### Lösung: WireGuard-Overlay-Mesh

Alle Nodes (Coordinator + Worker) bilden ein verschlüsseltes Overlay-Netzwerk:

```
Vorteile:
  ✓ NAT-Traversal automatisch (UDP-Hole-Punching)
  ✓ Stabile virtuelle IPs pro Node (10.42.0.0/16 z.B.)
  ✓ Verschlüsselung End-to-End (ChaCha20-Poly1305)
  ✓ NCCL sieht es als "lokales Netzwerk" → funktioniert direkt
  ✓ ~1-2% CPU-Overhead, vernachlässigbare Latenz im LAN-Fall
```

**Implementierung:**
- **Hub:** [Headscale](https://github.com/juanfont/headscale) (Open-Source-Tailscale-Coordinator) im Backend-Image, oder eigener Rust-basierter WG-Coordinator.
- **Clients:** WireGuard-Kernel-Modul (Linux) bzw. wireguard-go (Win/macOS), automatisch konfiguriert vom Bootstrapper nach Enrollment.
- **Subnet:** `10.42.0.0/16` für Cluster, jede Node bekommt feste IP (z.B. `10.42.0.5`).

### WAN-Realität: Bandbreite & Latenz

| Szenario | Latenz | Bandbreite | Empfohlene Strategie |
|---|---|---|---|
| Alle im LAN | <1ms | 1–10 GbE | TP + PP, beides möglich |
| Standorte gleiche Stadt | 5–15ms | 100–1000 Mbps | PP + begrenzte TP |
| Region-übergreifend | 20–60ms | 50–500 Mbps | **nur PP** (Pipeline) |
| Interkontinental | 80–200ms | variabel | nur Single-Node-Jobs |

→ Scheduler **misst** RTT zwischen Nodes (Phase 1) und **partitioniert** das Cluster nach Latenz-Inseln. TP nur innerhalb Insel, PP zwischen Inseln.

### Optional: Direkte WireGuard-Peerings

Für Worker im selben LAN: Headscale erkennt das automatisch und routet direkt (P2P) statt durch den Hub → volle LAN-Bandbreite zwischen lokal benachbarten Workern.

---

## 5. Auto-Enrollment über Internet

Ziel: Ein User installiert den Bootstrapper, gibt EIN Geheimnis ein, fertig. Alles andere läuft automatisch.

### 5.1 Enrollment-Flow

```
   ┌─ ADMIN ─────────────────┐
   │ Web-UI → "Add Node"     │
   │ → generiert Enroll-Token│  (one-time-use, 15min TTL)
   │ → zeigt: TOKEN + URL    │
   └─────────────┬───────────┘
                 │ (paste/QR)
                 ▼
   ┌─ USER auf Worker-Host ──┐
   │ gpucluster-agent enroll │
   │   --token <TOKEN>       │
   │   --backend https://... │
   └─────────────┬───────────┘
                 │
   ┌─────────────▼─────────────────────────────────────────┐
   │ Bootstrapper:                                         │
   │  1. generiert Ed25519-Keypair (privater Key bleibt    │
   │     auf Host, idealerweise im TPM/Keystore)           │
   │  2. erfasst Hardware-Fingerprint (MAC+CPU+Board)      │
   │  3. POST /enroll  { token, pubkey, hw_fingerprint,    │
   │                     os_info, gpu_info }               │
   └─────────────┬─────────────────────────────────────────┘
                 │ TLS 1.3
                 ▼
   ┌─ GATEWAY → MGMT-BACKEND ─────────────────────────────┐
   │  - validiert Token (one-time, signed, TTL)           │
   │  - vergibt UUIDv7 als node_id                        │
   │  - signiert kurzlebiges Client-Cert (mTLS) für Node  │
   │    (CA: interne Backend-CA, Cert TTL 7d, auto-renew) │
   │  - generiert WireGuard-Peer-Config (priv. IP, hub-pk)│
   │  - schreibt Audit-Log: NODE_ENROLLED                 │
   │  - Antwort: { node_id, client_cert, ca_chain,        │
   │              wg_config, coord_endpoint }             │
   └─────────────┬────────────────────────────────────────┘
                 │
   ┌─────────────▼─────────────────────────────────────────┐
   │ Bootstrapper:                                         │
   │  - persistiert /var/lib/gpucluster/{node.id, key,     │
   │    cert, wg.conf}  (Disk-Encryption empfohlen)       │
   │  - aktiviert WireGuard                               │
   │  - pulled Worker-Image (richtiger CUDA-Tag)          │
   │  - startet Worker-Container                          │
   │  - Worker verbindet sich via mTLS zu Coordinator     │
   └───────────────────────────────────────────────────────┘
```

### 5.2 Sicherheits-Eigenschaften

- **Enroll-Token:** signiert vom Backend, one-time-use, 15min TTL → kein Token-Replay.
- **Cert-Lifetime:** kurz (7 Tage), automatische Renewal via Coordinator. Kompromittierte Node-Keys sind schnell wirkungslos.
- **Optional: 2-Mann-Regel:** Sensible Cluster können verlangen, dass ein Admin im UI die Node nochmal explizit *approved*, bevor sie Jobs annimmt.
- **Hardware-Fingerprint:** Verhindert, dass identisches Cert auf anderer Hardware genutzt wird (Detection bei Mismatch).
- **TPM/Secure-Element:** Wo verfügbar (Win11 mit TPM2.0, Linux mit TPM-Modul) wird privater Key dort generiert und nie exportiert.

### 5.3 Auto-Connect & Persistenter Service

Nach dem Enrollment muss sich der Worker **bei jedem Boot automatisch** mit dem Gateway verbinden — ohne User-Interaktion, auch nach Strom-/Internet-Ausfall.

**Linux (systemd):**

```ini
# /etc/systemd/system/gpucluster-agent.service
[Unit]
Description=GPU Cluster Agent
After=network-online.target docker.service
Wants=network-online.target
Requires=docker.service

[Service]
Type=notify
ExecStart=/usr/local/bin/gpucluster-agent run
Restart=always
RestartSec=5
WatchdogSec=30
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
```

**Windows:** Bootstrapper installiert sich als Windows Service (via `sc.exe` / `windows-service-rs`), startet mit Boot.

**Connection-Lifecycle:**

```
Boot
 │
 ▼
Bootstrapper Service start
 │
 ▼
Lade lokale Identität (node.id, cert, wg.conf) aus /var/lib/gpucluster/
 │
 ├─► Prüfe Cert-Ablauf < 48h?  → triggere Cert-Renewal
 │
 ▼
WireGuard-Tunnel hochfahren (wg-quick up)
 │
 ▼
TLS+mTLS-Verbindung zu wss://gateway/cluster/ws
 │
 ├─► Initial-Hello: NodeInfo (inkl. aktuelle WAN-IP, Driver, GPUs)
 │
 ▼
Persistente WebSocket-/gRPC-Stream-Connection
 ├─► Heartbeat alle 5s
 ├─► Empfängt Job-Dispatch-Messages
 └─► Empfängt Config-Push (Image-Update, Settings)

Bei Verbindungsabbruch:
 ├─► Exponential Backoff: 1s, 2s, 4s, 8s ... max 60s
 ├─► WG-Tunnel zyklisch neu (falls IP gewechselt)
 ├─► Re-Resolve DNS des Gateways
 └─► Bei 24h-Ausfall: Lokal cachen, Health weiter sammeln
```

**DNS-basierter Discovery:**
- Gateway-Endpoint ist eine Domain (`cluster.example.com`) — überlebt Backend-IP-Wechsel.
- Optional: SRV-Record für mehrere Gateway-Replicas (Failover).

**Resilienz-Eigenschaften:**
- ✓ Worker erholt sich allein von Crash, Reboot, Internet-Ausfall, IP-Wechsel beim ISP.
- ✓ Backend-Wartung (5min Downtime) → Worker reconnecten automatisch ohne Eingriff.
- ✓ Wenn Cert abläuft (z.B. nach 30d offline) → Bootstrapper triggert Re-Enrollment-Flow (Admin muss approven, Audit-Eintrag).

**Watchdog:** systemd Watchdog (30s) tötet hängende Agents → sofortiger Restart. Auf Windows äquivalent über Service-Recovery-Settings.

### 5.4 Re-Enrollment / Revocation

- Admin kann Node im Mgmt-UI revoken → Cert auf CRL → Coordinator weist Verbindungen ab.
- Node, die offline war > 30 Tage → automatisch quarantäniert, manuelle Re-Approval nötig.
- Hardware-Fingerprint-Änderung (z.B. neue GPU eingebaut) → Re-Enrollment-Event, Admin muss approven.

---

## 6. Gateway: Hochsicheres Edge

Der Gateway ist die **einzige nach außen exponierte Komponente** des Backends. Alles geht hier durch.

### 6.1 Defense-in-Depth-Layer

```
Internet
    │
    ▼
┌──────────────────────────────────────────────────────────┐
│ L1: TCP/UDP-Layer                                         │
│   - SYN-Cookies, Connection-Limits per IP                 │
│   - fail2ban-äquivalent (Auto-Ban nach N failed auth)     │
├──────────────────────────────────────────────────────────┤
│ L2: TLS-Termination                                       │
│   - TLS 1.3 ONLY, modern cipher suites                    │
│   - HSTS, OCSP-Stapling, Cert-Pinning für Clients         │
│   - Let's Encrypt o. eigene CA (rotiert)                  │
├──────────────────────────────────────────────────────────┤
│ L3: Authentifizierung                                     │
│   - mTLS für Worker-Nodes (Cert vom Enrollment)           │
│   - API-Tokens für externe Apps (LM Studio)               │
│     · gehasht in DB (Argon2id)                            │
│     · Scoped (read/write/admin)                           │
│     · Rotierbar, expirable                                │
│   - OAuth2/OIDC für Web-UI-User                           │
├──────────────────────────────────────────────────────────┤
│ L4: Authorization (RBAC)                                  │
│   - Rollen: admin, operator, user, viewer                 │
│   - Per-Resource-Permissions                              │
│   - Quota-Enforcement (Tokens/Hour, GPU-Hours/Month)      │
├──────────────────────────────────────────────────────────┤
│ L5: Rate-Limiting                                         │
│   - Sliding-Window, per IP + per Token + per User         │
│   - Burst-Protection (Token-Bucket)                       │
│   - Adaptive: erhöht bei Anomalie-Detection               │
├──────────────────────────────────────────────────────────┤
│ L6: Input-Validation / WAF                                │
│   - Strenges Schema-Validation (alle Endpoints)           │
│   - Max-Body-Size, Max-Header-Count                       │
│   - Prompt-Injection-Heuristiken (optional)               │
│   - SQLi/XSS-Filter (auch wenn Backend Prepared           │
│     Statements nutzt — Defense in Depth)                  │
├──────────────────────────────────────────────────────────┤
│ L7: Routing & Audit                                       │
│   - Weiterleitung an interne Services (mTLS)              │
│   - Vollständiger Audit-Log (immutable, append-only)      │
│   - Anomaly-Detection (ungewöhnliche Patterns)            │
└──────────────────────────────────────────────────────────┘
    │
    ▼
Interne Services (Coordinator, Mgmt-Backend, OpenAI-API)
```

### 6.2 Konkrete Implementierung (Rust)

```rust
// Gateway = axum + tower middleware stack
let app = Router::new()
    .route("/enroll", post(enrollment_handler))
    .nest("/v1", openai_compat_routes())
    .nest("/api", management_routes())
    .nest("/cluster", coordinator_routes())  // mTLS only
    .layer(tower::ServiceBuilder::new()
        .layer(SetSensitiveHeadersLayer::new(headers))
        .layer(TraceLayer::new_for_http())
        .layer(AuditLogLayer::new(audit_sink))
        .layer(RequestIdLayer::new())
        .layer(ConcurrencyLimitLayer::new(10_000))
        .layer(TimeoutLayer::new(Duration::from_secs(300)))
        .layer(RequestBodyLimitLayer::new(50 * 1024 * 1024))
        .layer(RateLimitLayer::sliding_window(...))
        .layer(SecurityHeadersLayer::strict())
        .layer(CorsLayer::very_permissive_for_lm_studio())
        .layer(AuthLayer::auto_select(jwt | api_key | mtls))
    );
```

Bibliotheken:
- `axum` + `tower` + `tower-http` (Middleware-Stack)
- `rustls` (TLS, kein OpenSSL → kleiner Angriffsvektor)
- `rcgen` + `x509-parser` (interne CA, Cert-Mgmt)
- `argon2` (Token-Hashing)
- `governor` (Rate-Limiting)
- `tracing` + `opentelemetry` (Audit + Observability)

### 6.3 Zero-Trust-Prinzipien

- **Keine impliziten Trust-Relations** — auch interne Services sprechen mTLS untereinander.
- **Least Privilege** — Worker-Cert kann *nur* Worker-API, keine Admin-Endpoints.
- **Continuous Verification** — Cert-Renewal alle 7 Tage zwingt zur Re-Validation.
- **Network-Segmentation** — Backend hinter Reverse-Proxy, nur Gateway öffentlich.

### 6.4 Audit-Log-Anforderungen

Jede sicherheitsrelevante Aktion wird unveränderlich protokolliert:

```
- ENROLLMENT_TOKEN_GENERATED  (admin_user, target_node_hint)
- ENROLLMENT_TOKEN_USED       (token_id, hw_fingerprint, ip, success)
- NODE_REGISTERED             (node_id, hw_fp, ip)
- NODE_REVOKED                (node_id, by_user, reason)
- AUTH_SUCCESS / AUTH_FAILURE (subject, ip, mechanism, reason)
- API_KEY_CREATED / REVOKED   (key_id, scope, by_user)
- JOB_SUBMITTED / COMPLETED   (job_id, user, model, gpus)
- ADMIN_ACTION                (any user/role/quota change)
- RATE_LIMIT_HIT              (subject, endpoint, count)
- SECURITY_ANOMALY            (type, severity, details)
```

Storage: append-only-Tabelle in Postgres + optionaler Export an SIEM (S3/Loki/Splunk).

---

## 7. Management-Backend (Komplette Verwaltung)

Eigenständiger Service hinter dem Gateway. Verwaltet alles **außer** dem reinen Compute-Scheduling (das ist Coordinator-Aufgabe).

### 7.1 Funktionsbereiche

```
┌─────────────────────────────────────────────────┐
│ Identity & Access                               │
│  - Users, Roles (admin/operator/user/viewer)    │
│  - OAuth2/OIDC + Local Auth + 2FA (TOTP)        │
│  - API-Key-Mgmt (create/rotate/revoke/scope)    │
│  - Session-Mgmt                                 │
├─────────────────────────────────────────────────┤
│ Node-Management                                 │
│  - Enrollment-Token-Generierung                 │
│  - Node-Approval-Workflow                       │
│  - Drain / Quarantine / Revoke                  │
│  - Tags & Labels (z.B. "office-fr", "highend")  │
│  - Live-Health-Status, Treiber-Versions-Drift   │
├─────────────────────────────────────────────────┤
│ Resource & Quota                                │
│  - Per-User / Per-Org GPU-Hour-Quotas           │
│  - Cost-Tracking (für Multi-Tenant Setups)      │
│  - Priority-Tiers (interactive/batch)           │
├─────────────────────────────────────────────────┤
│ Model-Registry                                  │
│  - GGUF/Safetensors-Models hochladbar           │
│  - Versionierung, Tagging                       │
│  - Auto-Distribution: welche Models auf welche  │
│    Nodes vorgewärmt werden sollen               │
├─────────────────────────────────────────────────┤
│ Dataset-Registry (Fine-Tuning)                  │
│  - Dataset-Upload, Privacy-Tagging              │
│  - Geo-Constraints (Daten dürfen z.B. nur in    │
│    EU-Worker-Nodes verarbeitet werden)          │
├─────────────────────────────────────────────────┤
│ Job-Management                                  │
│  - Inference-Endpoints (welche Models live)     │
│  - Fine-Tuning-Jobs (Spec, Status, Logs, Output)│
│  - Job-History, Retry, Cancel                   │
├─────────────────────────────────────────────────┤
│ Observability                                   │
│  - Prometheus-Metrics-Aggregation               │
│  - Grafana-Embedded-Dashboards                  │
│  - Audit-Log-Viewer                             │
│  - Alerting (Slack/Email/Webhook)               │
├─────────────────────────────────────────────────┤
│ Admin / Settings                                │
│  - Cluster-weite Settings                       │
│  - CA-Mgmt, Cert-Rotation                       │
│  - Backup/Restore                               │
└─────────────────────────────────────────────────┘
```

### 7.2 API-Design (REST + gRPC)

- **REST/JSON:** für Web-UI und externe Integrationen
- **gRPC:** für interne Service-zu-Service Kommunikation (Coordinator ↔ Mgmt)
- **WebSocket:** für Live-Logs / Job-Updates ans UI

### 7.3 Datenmodell (Auszug)

```
users(id, email, password_hash, totp_secret, created_at, ...)
roles(id, name, permissions)
user_roles(user_id, role_id)

api_keys(id, user_id, hash, scope, expires_at, last_used_at)

orgs(id, name, quota_gpu_hours, ...)
org_members(org_id, user_id, role)

nodes(id, hw_fingerprint, hostname, display_name, owner_user_id,
      org_id, status, first_seen, ...)
node_gpus(node_id, idx, uuid, name, arch, vram_bytes, driver_version, ...)
node_history(node_id, ts, event, details)

models(id, name, version, format, size_bytes, s3_url, ...)
model_distributions(model_id, node_id, status)

datasets(id, name, size_bytes, privacy_tags, geo_restriction, ...)

jobs(id, type, user_id, org_id, model_id, dataset_id,
     spec_yaml, status, submitted_at, started_at, completed_at, ...)
job_assignments(job_id, node_id, role)  -- pipeline-stage etc.
job_logs(job_id, ts, level, message)

audit_log(id, ts, actor, action, resource, ip, details_json)
```

### 7.4 Web-UI

- **Tech:** SvelteKit (klein, schnell) ODER Next.js (größeres Ökosystem) — Entscheidung in Phase 5.
- **Hauptansichten:**
  - Cluster-Overview (Live-Topologie, GPU-Auslastung)
  - Node-Detail (siehe §5 CLI-Beispiel, plus Steuerung)
  - Jobs (Liste, Detail, Logs, neue Inference/FT erstellen)
  - Models / Datasets
  - Users / API-Keys / Audit-Log
  - Settings / CA-Mgmt

---

## 8. Scheduler-Logik

```
Task kommt rein → Charakterisieren:
  - Modellgröße (z.B. 70B Q4 = ~40GB VRAM)
  - Task-Typ (Inference/Fine-Tune)
  - Latenz-Anforderung
  - Geo-Constraints (Dataset-Privacy etc.)

Scheduler entscheidet:
  ┌─ Passt auf 1 GPU?     → Single-Worker-Placement
  ├─ Passt auf 1 Node?    → Tensor-Parallel innerhalb Node
  ├─ Passt in 1 Latenz-Insel?
  │                        → TP+PP innerhalb Insel
  └─ Cross-Region nötig?  → nur Pipeline-Parallel
                            (Bandbreite/Latenz limitiert)

Heterogene GPUs: VRAM- und compute-score-gewichtete Layer-Verteilung
```

---

## 9. Node-Identifikation & Inventar

### 9.1 Client-ID-Vergabe (siehe §5.1)

UUIDv7 + Hardware-Fingerprint, persistiert in `/var/lib/gpucluster/node.id` (Volume-Mount im Container).

### 9.2 Erfasste Metadaten (gRPC/Protobuf)

```protobuf
message NodeInfo {
  string node_id          = 1;  // UUIDv7
  string hostname         = 2;
  string display_name     = 3;
  string hw_fingerprint   = 4;
  string owner_user_id    = 5;
  repeated string tags    = 6;  // "office-fr", "highend", ...

  OsInfo os               = 7;
  repeated GpuInfo gpus   = 8;
  NetworkInfo network     = 9;
  CpuMemInfo cpu_mem      = 10;
  GeoInfo geo             = 11; // approximate location, opt-in

  NodeStatus status       = 12;
  int64 first_seen        = 13;
  int64 last_heartbeat    = 14;
  string agent_version    = 15;
  string client_cert_sha  = 16; // für Audit
}

message NetworkInfo {
  // WAN / Internet
  string public_ip_v4         = 1;   // gemessen am Gateway (vom TCP-Socket)
  string public_ip_v6         = 2;   // optional, falls vorhanden
  string asn                  = 3;   // z.B. "AS3320 Deutsche Telekom"
  string isp                  = 4;
  string country_code         = 5;   // ISO, aus GeoIP
  string city                 = 6;   // approx, GeoIP
  bool   public_ip_is_dynamic = 7;   // erkannt durch Historie
  int64  public_ip_changed_at = 8;

  // LAN (vom Host gemeldet)
  repeated string local_ips   = 9;   // 192.168.1.42, 10.0.0.5, ...
  string primary_iface        = 10;  // "eth0" / "Ethernet 2"
  uint32 link_speed_mbps      = 11;  // 1000 / 10000

  // Cluster-Overlay
  string wg_ip                = 12;  // 10.42.0.5
  string wg_pubkey_sha        = 13;
  uint32 wg_listen_port       = 14;

  // Latenz
  uint32 rtt_to_gateway_ms    = 15;
  // pro-Peer-RTTs in separater RttMatrix-Message
}

message GpuInfo {
  uint32 index            = 1;
  string uuid             = 2;
  string name             = 3;  // "NVIDIA GeForce RTX 5060 Ti"
  string architecture     = 4;  // "Blackwell" / "Ada" / "Ampere"
  uint32 compute_cap_major= 5;
  uint32 compute_cap_minor= 6;
  uint64 vram_total_bytes = 7;
  uint64 vram_free_bytes  = 8;
  string pci_bus_id       = 9;
  string driver_version   = 10; // "566.36"
  string cuda_version     = 11; // "12.8"
  string vbios_version    = 12;
  uint32 power_limit_w    = 13;
  bool   nvlink_present   = 14;
}
```

### 9.3 Daten-Quellen

| Datenpunkt | Windows | Linux |
|---|---|---|
| OS-Info | `GetVersionEx` / WMI | `/etc/os-release` + `uname` |
| GPU + Treiber | **NVML** (`nvmlDeviceGet*`) | NVML (gleich) |
| CUDA Runtime | `cudaDriverGetVersion` | gleich |
| Mainboard/MAC | WMI | `dmidecode` / `/sys/class/net` |

→ Crate `nvml-wrapper`, gebündelt in `crates/sysinfo/`.

### 9.4 Lifecycle-States

```
PENDING_APPROVAL → Enrolled, aber Admin-Approval ausstehend
ONLINE           → Heartbeat < 10s alt
DEGRADED         → Heartbeat verspätet ODER GPU-Fehler
DRAINING         → Wird heruntergefahren
OFFLINE          → > 60s kein Heartbeat
QUARANTINED      → Treiber-Mismatch / NCCL-Inkompat / Security
REVOKED          → Cert revoked, dauerhaft blockiert
```

### 9.5 WAN-IP-Tracking & Dynamic-IP-Handling

Public IPs der Worker sind wichtig für:
- **Diagnostik / Support** (welche Node ist gerade von wo online)
- **Geo-Constraints** (Datasets dürfen nur in EU bleiben)
- **Anomalie-Detection** (plötzlich anderes Land → potentieller Compromise)
- **Direct-Peering-Optimierung** (Worker im selben /24 → P2P statt VPN-Hub)
- **ISP-/Netz-Auslastungs-Auswertung** (Bandbreiten-Limits)

**Erfassungs-Methode:**

Der Gateway sieht beim TLS-Handshake immer die echte Public-IP des Connect-Sockets — auch bei NAT auf User-Seite. Das ist die **autoritative Quelle**, nicht Self-Reporting des Workers.

```
Worker-TCP-Connect → Gateway nimmt remote_addr aus Socket
                  → schreibt in nodes.last_public_ip
                  → cross-check mit GeoIP → ASN, Country, City
                  → schreibt jeden Wechsel in node_ip_history
```

**Datenmodell (Erweiterung):**

```sql
ALTER TABLE nodes ADD COLUMN current_public_ip_v4 INET;
ALTER TABLE nodes ADD COLUMN current_public_ip_v6 INET;
ALTER TABLE nodes ADD COLUMN current_asn TEXT;
ALTER TABLE nodes ADD COLUMN current_country CHAR(2);
ALTER TABLE nodes ADD COLUMN public_ip_first_seen TIMESTAMPTZ;
ALTER TABLE nodes ADD COLUMN public_ip_last_changed TIMESTAMPTZ;

CREATE TABLE node_ip_history (
  id BIGSERIAL PRIMARY KEY,
  node_id UUID REFERENCES nodes(id),
  ts TIMESTAMPTZ DEFAULT now(),
  public_ip_v4 INET,
  public_ip_v6 INET,
  asn TEXT,
  country CHAR(2),
  city TEXT,
  source TEXT  -- 'tls_socket' | 'self_report'
);
```

**Dynamic-IP-Erkennung:**

```
Heuristik: Wenn IP > 3× pro Woche wechselt UND ASN konstant bleibt
        → public_ip_is_dynamic = true (typischer Heimanschluss)

Wenn ASN wechselt (z.B. AS3320 → AS6805):
  → User reist? Mobiles Tethering? → evtl. Re-Approval prompten
  → Audit-Event: NODE_IP_NETWORK_CHANGE

Wenn Country-Code wechselt:
  → Audit-Event: NODE_GEO_CHANGE (severity: medium)
  → Bei sensiblen Datasets → Job-Abbruch / Drain
  → Mgmt-UI zeigt Warning
```

**GeoIP-Quelle:** MaxMind GeoLite2 (kostenlos, lokal als DB im Backend, monatliches Update via Job).

**Privacy-Aspekt:** Public IPs sind PII in EU. Mgmt-Backend bietet:
- Aufbewahrungsdauer konfigurierbar (default 90 Tage in `node_ip_history`)
- Owner kann eigene IP-History sehen, andere User nicht
- Admin-Zugriff auf IPs immer im Audit-Log

**CLI-Anzeige:**

```
$ gpucluster nodes show 018fbb...e3a1
...
  Network:
    Public:    84.123.45.67  (DE, AS3320 Deutsche Telekom, dynamisch)
               first seen 2026-04-01, last change 2026-04-22 (~3w stable)
    LAN:       192.168.1.42 (eth0, 10000 Mbps)
    Cluster:   wg 10.42.0.5 — RTT to gw: 24ms
    History:   12 IP changes in last 30d (typical home connection)
...
```

**Mgmt-API-Endpunkte:**

```
GET  /api/nodes/{id}/network       → aktuelle Werte
GET  /api/nodes/{id}/ip-history    → Liste mit Pagination
GET  /api/cluster/topology         → alle Nodes mit Public-IP-Locations (für Karte)
```

### 9.6 CLI-Ansicht

```
$ gpucluster nodes list
ID                NAME              OS              GPUs           STATUS    SITE
018fbb...e3a1     workstation-dani  Win 11 24H2     1× RTX 5060Ti  ONLINE    home-fr
018fbc...77f2     ai-rig-01         Ubuntu 24.04    2× RTX 4090    ONLINE    office-de
018fbd...91c4     homelab-tower     Win 10 22H2     1× RTX 3090    QUARANTI. home-de

$ gpucluster nodes show 018fbb...e3a1
Node: workstation-dani (018fbb...e3a1)
  Owner:     dzurmuehle@gmail.com    Site: home-fr
  OS:        Windows 11 24H2 (10.0.26100, x86_64)
  Agent:     v0.1.0
  Network:   public 84.x.x.x → wg 10.42.0.5  ·  RTT: 24ms
  CPU/RAM:   Ryzen 9 7950X · 64 GB DDR5
  GPUs:
    [0] RTX 5060 Ti (Blackwell, sm_120)
        VRAM:    16384 MB (15820 MB free)
        Driver:  566.36       CUDA: 12.8
        VBIOS:   95.06.2F.00.A1
        Power:   180W limit
  Cert:      sha256:af3b... (renews in 5d)
  First seen:    2026-04-29 09:12:03
  Last heartbeat: 2026-04-29 14:33:51 (2s ago)
```

---

## 10. Heterogenes-Cluster-Design (First-Class)

### 10.1 GPU-Capability-Profile

```rust
struct GpuCapabilityProfile {
    architecture: Arch,
    compute_cap: (u32, u32),
    vram_bytes: u64,
    mem_bandwidth_gbs: f32,
    supports_fp16:  bool,
    supports_bf16:  bool,
    supports_fp8:   bool,
    supports_fp4:   bool,
    supports_int8_tc: bool,
    benchmark_score: BenchScore,
}
```

Bench beim Join: kleiner Reference-Workload (Llama-3.2-1B forward pass) → gemessene statt nur spezifizierte Performance.

### 10.2 Layer-Allocation (Pipeline Parallelism)

VRAM-/Score-gewichtete Verteilung (Greedy in Phase 2, MILP via `good_lp` später).

### 10.3 Tensor-Parallel nur in homogenen Gruppen

Cluster wird in Compute-Groups partitioniert. TP innerhalb, PP zwischen.

### 10.4 Numerik-Mismatch

Default BF16 (überall verfügbar). User kann pro-Job höhere Präzision (FP8/FP4) erzwingen → Scheduler nimmt nur kompatible GPUs.

### 10.5 CUDA-Binary

```cmake
set(CMAKE_CUDA_ARCHITECTURES 86 89 90 120)
```

Fat Binary deckt Ampere/Ada/Hopper/Blackwell ab.

### 10.6 NCCL-Tuning für WAN

```
NCCL_ALGO=Ring
NCCL_P2P_DISABLE=0      # innerhalb Latenz-Insel
NCCL_NET_GDR_LEVEL=0    # WAN: kein GPUDirect über VPN
NCCL_SOCKET_IFNAME=wg0  # über WireGuard
NCCL_IB_DISABLE=1       # keine InfiniBand verfügbar
```

Auto-Tuning per Cluster beim Bootstrap.

---

## 11. Docker / Image-Strategie

### 11.1 Backend-System-Image

```yaml
# backend/docker-compose.yml (vereinfacht)
services:
  gateway:        image: gpucluster/gateway:0.1.0
  coordinator:    image: gpucluster/coordinator:0.1.0
  mgmt:           image: gpucluster/mgmt:0.1.0
  openai-api:     image: gpucluster/openai-api:0.1.0
  wg-hub:         image: headscale/headscale:latest
  postgres:       image: postgres:16
  redis:          image: redis:7
  minio:          image: minio/minio
  caddy:          image: caddy:2          # auto-TLS, reverse proxy
```

→ `docker compose up -d` → komplettes Backend läuft. Updates: `git pull && docker compose up -d`.

Für Production: Helm-Chart (Phase 5).

### 11.2 Worker-Container (Client-Seite)

```
gpucluster/worker:0.1.0-cuda12.8       ~6 GB   (CUDA + cuDNN + NCCL + llama.cpp + Worker)
gpucluster/worker:0.1.0-cuda12.4       für ältere Treiber
gpucluster/worker:0.1.0-cuda11.8       legacy
```

Bootstrapper wählt automatisch passenden Tag.

### 11.3 Worker-Dockerfile (Skelett)

```dockerfile
FROM nvidia/cuda:12.8.0-cudnn-devel-ubuntu24.04 AS build
RUN apt-get update && apt-get install -y curl build-essential cmake git \
    && curl https://sh.rustup.rs -sSf | sh -s -- -y
COPY . /src
WORKDIR /src
RUN cargo build --release -p worker \
 && cmake -B cpp/build cpp/llama-rpc-ext && cmake --build cpp/build -j

FROM nvidia/cuda:12.8.0-cudnn-runtime-ubuntu24.04
COPY --from=build /src/target/release/worker /usr/local/bin/
COPY --from=build /src/cpp/build/llama-rpc-server /usr/local/bin/
ENTRYPOINT ["/usr/local/bin/worker"]
```

### 11.4 Worker docker-compose (auf User-Host)

```yaml
services:
  worker:
    image: gpucluster/worker:0.1.0-cuda12.8
    runtime: nvidia
    network_mode: host
    ipc: host
    cap_add: [NET_ADMIN]            # für WireGuard
    ulimits:
      memlock: -1
      stack: 67108864
    volumes:
      - /var/lib/gpucluster:/var/lib/gpucluster   # node.id, certs, wg.conf
    environment:
      - COORDINATOR_URL=https://cluster.example.com
    deploy:
      resources:
        reservations:
          devices:
            - driver: nvidia
              count: all
              capabilities: [gpu]
```

### 11.5 Windows: WSL2-Konfiguration

```ini
# %USERPROFILE%\.wslconfig
[wsl2]
networkingMode=mirrored
firewall=true
dnsTunneling=true
```

→ Mirrored Networking macht WireGuard-Tunnel auf WSL-Seite vom Internet aus erreichbar.

### 11.6 Bootstrapper

```
gpucluster-agent.exe   (Windows, ~5 MB Rust-Binary, läuft als Windows Service)
gpucluster-agent       (Linux, ~5 MB Rust-Binary, läuft als systemd-Unit)

Aufgaben:
  - Pre-Flight: Docker installiert? WSL2 (Win)? GPU sichtbar?
  - Enrollment-Flow (einmalig, siehe §5.1)
  - **Service-Installation** (systemd / Windows Service) → Auto-Start bei Boot
  - WireGuard-Setup + Auto-Reconnect
  - **Persistente Connection zum Gateway** (mTLS, Reconnect mit Backoff)
  - Image-Pull mit korrektem CUDA-Tag
  - Container-Lifecycle (start/stop/restart/update)
  - Auto-Update auf Coordinator-Push
  - Health-Reporting (Host-OS, Driver, WAN-IP — sieht Container nicht)

Install-Flow (User-Sicht):

  $ gpucluster-agent install
    → erkennt OS, prüft Voraussetzungen
    → bietet ggf. Auto-Install von Docker/WSL2 an
    → registriert sich als systemd/Windows Service
    → Status: "Bereit für Enrollment"

  $ gpucluster-agent enroll --token <TOKEN>
    → vollzieht Enrollment-Flow
    → speichert Identity in /var/lib/gpucluster/
    → startet Service
    → ab jetzt automatisch bei jedem Boot

  $ gpucluster-agent status
    → Connection-Status, GPU-Info, letzter Heartbeat,
      aktuell laufende Jobs

  $ gpucluster-agent uninstall
    → drained Jobs, deregistriert beim Coordinator,
      entfernt Service + Container + Daten
```

---

## 12. Phasen-Roadmap

### Phase 0 — Foundation (~2 Wochen)
- [ ] Rust-Workspace-Setup (Cargo Workspaces)
- [ ] Backend-Docker-Compose (Gateway-Skelett, Coordinator-Skelett, Mgmt-Skelett, Postgres, Redis, MinIO, Caddy)
- [ ] Worker-Dockerfile + Multi-Tag-Build-Pipeline (CUDA 12.8/12.4/11.8)
- [ ] Bootstrapper-Skelett (Win/Linux)
- [ ] Lokale Container-Registry (oder GHCR)

### Phase 1 — Identity & Cluster-Fundament (3 Wochen)
- [ ] gRPC-Protokoll (tonic + protobuf): `NodeInfo`, `NetworkInfo`, `GpuCapabilityProfile`
- [ ] `crates/sysinfo`: NVML + OS-Detection (Win/Linux)
- [ ] Mgmt-Backend: Users, RBAC, API-Keys, Audit-Log (Postgres)
- [ ] Gateway: TLS 1.3, mTLS für Worker, JWT/OAuth für Web-UI
- [ ] Interne CA + Cert-Mgmt
- [ ] **Auto-Enrollment-Flow** (Token-Generation, Verify, Cert-Issue)
- [ ] WireGuard-Hub (Headscale integriert)
- [ ] Bootstrapper: enroll → wg-up → worker-start
- [ ] **Bootstrapper als systemd / Windows Service**
- [ ] **Persistente Auto-Reconnect-Connection** (Backoff, Watchdog)
- [ ] **WAN-IP-Tracking** (TLS-Socket-basiert, GeoIP-Lookup, History-Tabelle)
- [ ] Coordinator: Heartbeats, Node-Liste, Quarantäne bei Driver-Mismatch
- [ ] CLI: `gpucluster nodes list / show / approve / revoke`
- [ ] Capability-Profil-Erfassung + Bench-on-Join

### Phase 2 — Distributed Inference über WAN (4 Wochen)
- [ ] llama.cpp Submodule + RPC-Hooks
- [ ] Worker startet/managed lokalen `rpc-server`
- [ ] Latenz-Insel-Detection (RTT-Matrix zwischen Workern)
- [ ] **Heterogenitätsbewusster** Layer-Allocation-Solver (Greedy)
- [ ] Compute-Group-Partitionierung (TP nur homogen)
- [ ] OpenAI-API-Layer (`/v1/chat/completions`)
- [ ] Test: 70B-Modell gestreckt über 2 Standorte

### Phase 3 — Smart Scheduling & Observability (2-3 Wochen)
- [ ] Multi-Job-Queue (Priority + Fair-Share + Quotas)
- [ ] Geo-/Privacy-Constraints im Scheduler
- [ ] Dynamisches Re-Balancing
- [ ] NCCL-Bench beim Bootstrap → optimale ENV-Vars
- [ ] Prometheus-Export, Grafana-Dashboards
- [ ] Alerting (Webhook/Slack)
- [ ] Anomaly-Detection im Gateway

### Phase 4 — Fine-Tuning (4-5 Wochen)
- [ ] LoRA/QLoRA über `candle` (Rust) ODER PyTorch-Bridge
- [ ] DDP via NCCL-Wrapper
- [ ] FSDP für >Single-Node-Modelle
- [ ] Checkpoint-Sync, Output-Adapter-Mgmt
- [ ] Job-Spec YAML/TOML
- [ ] Dataset-Registry mit Privacy-Tags

### Phase 5 — Production-UI & Hardening (3-4 Wochen)
- [ ] Web-Dashboard (SvelteKit / Next.js)
- [ ] Live-Topologie, Job-Monitor, Logs
- [ ] Audit-Log-Viewer
- [ ] Helm-Chart für Backend-Deploy
- [ ] Auto-Updater für Worker-Image
- [ ] Penetration-Test des Gateways
- [ ] Backup/Restore, DR-Runbook

---

## 13. Repository-Struktur

```
MultiGPUCluster/
├── Cargo.toml                       (Rust Workspace)
├── crates/
│   ├── proto/                       (gRPC + Protobuf-Definitionen)
│   ├── common/                      (Shared Types, Errors)
│   ├── sysinfo/                     (NVML + OS-Detection)
│   ├── gateway/                     (Edge-Gateway, mTLS, WAF)
│   ├── coordinator/                 (Master-Service, Scheduler)
│   ├── scheduler/                   (Placement-Algorithmen, Bench)
│   ├── mgmt-backend/                (Users, RBAC, Audit, Quotas)
│   ├── openai-api/                  (LM Studio-Compat-Layer)
│   ├── worker/                      (Node-Agent, läuft im Container)
│   ├── nccl-wrapper/                (Rust-friendly NCCL FFI)
│   └── ca/                          (Interne CA, Cert-Issue/Renew)
├── cpp/
│   ├── llama-rpc-ext/               (llama.cpp Fork)
│   └── cuda-kernels/                (Custom Ops)
├── bootstrapper/                    (Native Host-Agent)
├── cli/                             (gpucluster CLI)
├── dashboard/                       (Web-UI: SvelteKit/Next.js)
├── backend/
│   ├── docker-compose.yml           (Backend-System-Image)
│   ├── Caddyfile                    (Reverse-Proxy / TLS)
│   └── helm/                        (Helm-Chart für K8s)
├── docker/
│   ├── gateway.Dockerfile
│   ├── coordinator.Dockerfile
│   ├── mgmt.Dockerfile
│   ├── openai-api.Dockerfile
│   └── worker.Dockerfile
└── docs/
    └── PLAN.md                      (dieses Dokument)
```

---

## 14. Kritische Voraussetzungen

| Thema | Empfehlung |
|---|---|
| **Backend-Hosting** | VPS/Cloud mit fester IP + Domain (für Gateway). Min. 4 vCPU / 8 GB / 100 GB SSD. Hetzner/Scaleway/Hyperstack billig. |
| **Backend-CA** | Self-Signed Root + Intermediate, Root offline aufbewahren. Let's Encrypt für externe TLS-Endpoint. |
| **Netzwerk Worker** | Beliebige Internet-Verbindung, NAT OK (WireGuard löst). Min. 50 Mbps Upload für Multi-Node-Inference brauchbar. |
| **CUDA** | RTX 5060 Ti = Blackwell (sm_120) → CUDA 12.8+. |
| **Treiber-Drift** | Coordinator quarantiert inkompatible Nodes automatisch. |
| **Windows-Hosts** | WSL2 mit `networkingMode=mirrored` zwingend. |
| **Secrets** | Backend nutzt `age`/`sops` o. Vault für Secrets-at-Rest. |
| **Backups** | Postgres + MinIO regelmäßig snapshotten (Daily off-site). |

---

## 15. Offene Fragen / Entscheidungen

1. **llama.cpp RPC ist experimentell** — Machbarkeitsstudie in Phase 2-Start.
2. **Fine-Tuning-Stack:** `candle` Rust-only oder hybrid mit PyTorch via `pyo3`? Entscheidung in Phase 4.
3. **WG-Hub:** Headscale (fertig) oder eigener Rust-Coordinator (mehr Kontrolle, mehr Code)?
4. **Web-UI-Framework:** SvelteKit (klein) vs. Next.js (Ökosystem)?
5. **Backend-Hosting:** Selbst-gehostet (VPS) oder von Anfang an Cloud-managed (z.B. Fly.io)?
6. **Hardware-Inventar:** Welche Worker-Nodes konkret? Standorte/Netzanbindung pro Node?

---

## 16. Nächste Schritte

1. Phase 0 starten: Rust-Workspace + Backend-Compose-Skelett + Worker-Dockerfile
2. Parallel: Bootstrapper-Skelett + Enrollment-Flow-Design
3. Hardware-Inventar dokumentieren (siehe §15.6)
4. Backend-Hosting entscheiden (§15.5) — bestimmt CI/CD-Setup
