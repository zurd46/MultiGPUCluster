#!/usr/bin/env bash
# Convenience: pulls a small GGUF model into ./models/ for end-to-end tests.
# Defaults to Llama-3.2-1B-Instruct Q4_K_M (~770 MB) — fits comfortably on
# every supported worker (RTX 3090 / 4090 / 5060 Ti / Apple M1+).
#
# Usage:
#   scripts/download-model.sh                  # default model
#   scripts/download-model.sh qwen2.5-0.5b     # named alias
#   scripts/download-model.sh --url <https...> # arbitrary GGUF URL
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p models

# Curated aliases — small enough to download on a coffee break, useful enough
# to actually validate the pipeline. Add more here, don't lie about sizes.
declare_alias() { ALIASES["$1"]="$2:$3"; }
declare -A ALIASES
declare_alias "llama-3.2-1b" \
    "https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf" \
    "Llama-3.2-1B-Instruct-Q4_K_M.gguf"
declare_alias "qwen2.5-0.5b" \
    "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf" \
    "qwen2.5-0.5b-instruct-q4_k_m.gguf"
declare_alias "tinyllama-1.1b" \
    "https://huggingface.co/TheBloke/TinyLlama-1.1B-Chat-v1.0-GGUF/resolve/main/tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf" \
    "tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf"

DEFAULT_ALIAS="llama-3.2-1b"

if [[ "${1:-}" == "--url" ]]; then
    URL="${2:?--url requires a URL}"
    FILE="$(basename "$URL")"
elif [[ -n "${1:-}" ]]; then
    ALIAS="$1"
    if [[ -z "${ALIASES[$ALIAS]:-}" ]]; then
        echo "unknown alias '$ALIAS'. Known: ${!ALIASES[*]}" >&2
        exit 1
    fi
    URL="${ALIASES[$ALIAS]%%:*}"
    FILE="${ALIASES[$ALIAS]##*:}"
else
    URL="${ALIASES[$DEFAULT_ALIAS]%%:*}"
    FILE="${ALIASES[$DEFAULT_ALIAS]##*:}"
fi

DEST="models/$FILE"
if [[ -f "$DEST" ]]; then
    echo "✓ already present: $DEST"
    echo
    echo "Use it with:  MODEL_PATH=\"\$PWD/$DEST\" ./target/release/gpucluster-worker"
    exit 0
fi

echo "==> Downloading $FILE"
echo "    from $URL"
echo "    to   $DEST"
curl --fail --location --progress-bar -o "$DEST.partial" "$URL"
mv "$DEST.partial" "$DEST"

echo
echo "✓ done: $(du -h "$DEST" | cut -f1)  $DEST"
echo
echo "Next steps:"
echo "  export MODEL_PATH=\"\$PWD/$DEST\""
echo "  ./target/release/gpucluster-worker --coordinator-url https://localhost/cluster --data-dir /tmp/gpucluster-worker"
