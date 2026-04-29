-- Phase 2: Hugging Face model integration.
--
-- The model registry now carries the source needed for an unattended download:
-- a HF repo id + the file inside that repo. Workers read these fields when
-- they receive a `load_model` control RPC and stream the GGUF straight from
-- the Hub into their local data-dir.
--
--   hf_repo        — owner/name on Hugging Face (e.g. "bartowski/Llama-3.2-1B-Instruct-GGUF").
--                    Empty string means "not from HF" (legacy, manual MODEL_PATH path).
--   hf_file        — file inside the repo (e.g. "Llama-3.2-1B-Instruct-Q4_K_M.gguf").
--                    Required when hf_repo is set; otherwise empty.
--   local_filename — what the worker should save it as on disk under data-dir/models/.
--                    Defaults to hf_file when empty so admins don't have to think about it.
--   loaded_on_node — opportunistic cache: which worker most recently loaded this model.
--                    Updated by the load handler as a soft hint for the dispatcher; the
--                    authoritative answer is what the worker reports in its heartbeat.

ALTER TABLE models
    ADD COLUMN IF NOT EXISTS hf_repo        TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS hf_file        TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS local_filename TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS loaded_on_node TEXT NOT NULL DEFAULT '';

-- The 'downloading' status was implicitly allowed by the handler in 0003; make
-- it explicit here so future migrations have a single source of truth for the
-- enum-via-text contract. (Phase 1 deliberately stayed off CHECK constraints
-- so the admin can backfill weird states during incident recovery.)
COMMENT ON COLUMN models.status IS
    'available | loading | downloading | disabled | error';

-- Seed the HF token slot in cluster_settings so the GET /settings response has
-- it from the start (returns "" when never set, so the UI input is empty).
INSERT INTO cluster_settings (key, value) VALUES
    ('huggingface_api_token', '""'::jsonb)
ON CONFLICT (key) DO NOTHING;
