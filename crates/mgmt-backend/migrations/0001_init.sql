-- Phase 1: initial schema
CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- Cluster-internal CA materials. Single row (id=1) for the active CA.
-- For higher security in production these should live in an HSM/KMS;
-- this table is the bootstrap path.
CREATE TABLE IF NOT EXISTS ca_state (
    id           INT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    common_name  TEXT NOT NULL,
    cert_pem     TEXT NOT NULL,
    key_pem      TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS users (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email           TEXT UNIQUE NOT NULL,
    display_name    TEXT NOT NULL DEFAULT '',
    password_hash   TEXT NOT NULL,
    totp_secret     TEXT,
    role            TEXT NOT NULL DEFAULT 'user',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS api_keys (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    hash            TEXT NOT NULL,
    scope           TEXT NOT NULL DEFAULT 'read',
    expires_at      TIMESTAMPTZ,
    last_used_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS enroll_tokens (
    id              UUID PRIMARY KEY,
    token_hash      TEXT NOT NULL UNIQUE,
    requested_by    UUID REFERENCES users(id),
    display_hint    TEXT,
    expires_at      TIMESTAMPTZ NOT NULL,
    used_at         TIMESTAMPTZ,
    used_by_node    UUID,
    used_from_ip    INET,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_enroll_tokens_hash ON enroll_tokens(token_hash);

CREATE TABLE IF NOT EXISTS nodes (
    id                       UUID PRIMARY KEY,
    hw_fingerprint           TEXT UNIQUE NOT NULL,
    hostname                 TEXT,
    display_name             TEXT,
    owner_user_id            UUID REFERENCES users(id),
    status                   TEXT NOT NULL DEFAULT 'pending_approval',
    agent_version            TEXT,
    pubkey_ed25519           BYTEA,
    client_cert_sha          TEXT,
    cert_expires_at          TIMESTAMPTZ,
    current_public_ip_v4     INET,
    current_public_ip_v6     INET,
    current_asn              TEXT,
    current_country          CHAR(2),
    public_ip_first_seen     TIMESTAMPTZ,
    public_ip_last_changed   TIMESTAMPTZ,
    first_seen               TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_heartbeat           TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS node_gpus (
    node_id          UUID NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    idx              INT NOT NULL,
    uuid             TEXT,
    name             TEXT,
    architecture     TEXT,
    compute_cap_major INT,
    compute_cap_minor INT,
    vram_total_bytes BIGINT,
    driver_version   TEXT,
    cuda_version     TEXT,
    PRIMARY KEY (node_id, idx)
);

CREATE TABLE IF NOT EXISTS node_ip_history (
    id           BIGSERIAL PRIMARY KEY,
    node_id      UUID NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    ts           TIMESTAMPTZ NOT NULL DEFAULT now(),
    public_ip_v4 INET,
    public_ip_v6 INET,
    asn          TEXT,
    country      CHAR(2),
    city         TEXT,
    source       TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_node_ip_history_node_ts ON node_ip_history(node_id, ts DESC);

CREATE TABLE IF NOT EXISTS audit_log (
    id        UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    ts        TIMESTAMPTZ NOT NULL DEFAULT now(),
    actor     TEXT NOT NULL,
    action    TEXT NOT NULL,
    resource  TEXT,
    ip        INET,
    details   JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_log(ts DESC);
CREATE INDEX IF NOT EXISTS idx_audit_actor ON audit_log(actor);
