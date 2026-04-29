-- Phase 1.x: customer-facing API keys for /v1/* (OpenAI-compatible inference).
--
-- The 0001 schema modelled api_keys as user-bound (auth tokens for the dashboard
-- once OAuth is wired up). For Phase 1 we also need *admin-minted* keys that
-- aren't tied to any user row — that's how `gpucluster keys create` issues a
-- bearer token to LM Studio. So:
--
--   * make `user_id` nullable (admin-minted keys have no user)
--   * add `name`     — human-readable label ("lm-studio-laptop")
--   * add `prefix`   — first 12 chars of the token, stored verbatim, so the UI
--                      can show "mgc_abcd…" without ever revealing the secret
--   * add `revoked_at` — soft-delete; we keep the row for audit
--   * scope default flips to 'inference' (the common case for these keys)

ALTER TABLE api_keys
    ALTER COLUMN user_id DROP NOT NULL;

ALTER TABLE api_keys
    ADD COLUMN IF NOT EXISTS name        TEXT        NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS prefix      TEXT        NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS revoked_at  TIMESTAMPTZ;

ALTER TABLE api_keys
    ALTER COLUMN scope SET DEFAULT 'inference';

-- Listing API by prefix is the hot path on the admin UI; index it.
CREATE INDEX IF NOT EXISTS idx_api_keys_prefix     ON api_keys(prefix);
CREATE INDEX IF NOT EXISTS idx_api_keys_revoked_at ON api_keys(revoked_at);
