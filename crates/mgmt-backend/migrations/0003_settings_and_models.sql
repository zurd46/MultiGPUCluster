-- Phase 1.x: cluster-level configuration that the admin UI can edit live.
--
-- Two flat tables:
--
--   cluster_settings — KV store for things that don't fit anywhere else.
--                      Today: public_base_url (the URL admins point clients
--                      at — e.g. https://cluster.example.com), default_model,
--                      rate_limit_rpm, max_tokens_default.
--                      JSONB value column so we can grow types without
--                      another migration.
--
--   models           — Models the cluster *advertises* on /v1/models. Phase 2
--                      will populate `status` from the live load state of
--                      llama.cpp on the workers; for now it's manually
--                      managed in the admin UI.
--
-- Both are admin-write, public-read for the relevant subset.

CREATE TABLE IF NOT EXISTS cluster_settings (
    key         TEXT PRIMARY KEY,
    value       JSONB NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by  TEXT NOT NULL DEFAULT 'admin'
);

-- Seed sane defaults so the UI has something to render on first load.
INSERT INTO cluster_settings (key, value) VALUES
    ('public_base_url',     '""'::jsonb),
    ('default_model',       '"auto"'::jsonb),
    ('rate_limit_rpm',      '60'::jsonb),
    ('max_tokens_default',  '4096'::jsonb)
ON CONFLICT (key) DO NOTHING;

CREATE TABLE IF NOT EXISTS models (
    id            TEXT PRIMARY KEY,
    display_name  TEXT NOT NULL DEFAULT '',
    description   TEXT NOT NULL DEFAULT '',
    -- 'available' | 'loading' | 'disabled' | 'error'
    -- Phase 2 will own this column; for Phase 1 it's manually set.
    status        TEXT NOT NULL DEFAULT 'available',
    is_default    BOOLEAN NOT NULL DEFAULT FALSE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Only one row may carry is_default=TRUE — but using a UNIQUE partial index
-- instead of a CHECK lets the UI flip the flag without a server-side dance.
CREATE UNIQUE INDEX IF NOT EXISTS uniq_models_default
    ON models((1)) WHERE is_default;
