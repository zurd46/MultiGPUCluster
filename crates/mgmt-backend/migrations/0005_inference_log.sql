-- One row per /v1/chat/completions (and later: /v1/completions, /v1/embeddings).
--
-- Written by openai-api right before it returns to the customer — covers both
-- success and every failure mode (no eligible node, worker unreachable, etc.)
-- so the admin UI can show a complete request history without operators
-- having to grep docker logs.
--
-- Storage policy is "keep recent, prune via a separate worker": we don't
-- enforce retention here. Admin UI queries with LIMIT 200 by default; a
-- cron job can DELETE WHERE created_at < now() - interval '30 days' once
-- volume justifies it.

CREATE TABLE IF NOT EXISTS inference_log (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Gateway's x-request-id, propagated through openai-api so logs across
    -- services correlate. NULL when the gateway didn't set one (direct hit
    -- on openai-api, dev only).
    request_id      TEXT        NULL,
    -- Endpoint the customer called. "chat.completions" today; "completions"
    -- and "embeddings" once they're routable.
    endpoint        TEXT        NOT NULL DEFAULT 'chat.completions',
    -- Model id from the request body. Free-form text — we record what the
    -- client asked for, not what we resolved. Useful for catching typos in
    -- LM Studio's "model" dropdown.
    model           TEXT        NOT NULL DEFAULT '',
    -- Worker that served the request. NULL means "dispatch failed before we
    -- picked one" (no eligible nodes, etc.).
    node_id         TEXT        NULL,
    inference_url   TEXT        NULL,
    -- Customer key prefix (mgc_xxxxxxxx) — never the full token, never the
    -- hash. Lets the admin attribute traffic to a key without leaking it.
    api_key_prefix  TEXT        NULL,
    -- Counts from llama-server's `usage` block when the response was 2xx.
    prompt_tokens     INT4 NULL,
    completion_tokens INT4 NULL,
    total_tokens      INT4 NULL,
    -- Wall-clock from openai-api receiving the request to it returning.
    latency_ms      INT4        NULL,
    -- HTTP status the customer got back.
    status_code     INT4        NOT NULL DEFAULT 0,
    -- One of: ok, no_eligible_nodes, no_inference_endpoint, worker_unreachable,
    -- worker_returned_error, worker_response_parse_error, coordinator_error,
    -- coordinator_unreachable. Distinct column from the message because it's
    -- enum-shaped — we filter on it in the UI.
    error_type      TEXT        NULL,
    error_message   TEXT        NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The two queries the UI runs: most-recent-first (default view) and
-- per-node breakdown (for "show me what worker X served").
CREATE INDEX IF NOT EXISTS idx_inference_log_created_at
    ON inference_log (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_inference_log_node_id
    ON inference_log (node_id, created_at DESC)
    WHERE node_id IS NOT NULL;
