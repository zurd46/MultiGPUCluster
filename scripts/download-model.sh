#!/usr/bin/env bash
# Convenience: pulls a small GGUF model into ./models/ for end-to-end tests.
# Defaults to Llama-3.2-1B-Instruct Q4_K_M (~770 MB) — fits comfortably on
# every supported worker (RTX 3090 / 4090 / 5060 Ti / Apple M1+).
#
# Usage:
#   scripts/download-model.sh                  # default model (llama-3.2-1b)
#   scripts/download-model.sh qwen2.5-0.5b     # named alias (smallest)
#   scripts/download-model.sh tinyllama-1.1b   # named alias
#   scripts/download-model.sh --url <URL> <FILE>
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p models

ALIAS="${1:-llama-3.2-1b}"

case "$ALIAS" in
    --url)
        URL="${2:?--url requires <url> <filename>}"
        FILE="${3:?--url requires <url> <filename>}"
        ;;
    llama-3.2-1b|"")
        URL="https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf"
        FILE="Llama-3.2-1B-Instruct-Q4_K_M.gguf"
        ;;
    qwen2.5-0.5b)
        URL="https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf"
        FILE="qwen2.5-0.5b-instruct-q4_k_m.gguf"
        ;;
    tinyllama-1.1b)
        URL="https://huggingface.co/TheBloke/TinyLlama-1.1B-Chat-v1.0-GGUF/resolve/main/tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf"
        FILE="tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf"
        ;;
    list|--list)
        cat <<'EOF'
Available aliases:
  llama-3.2-1b     Llama-3.2-1B-Instruct  Q4_K_M  ~770 MB  (default)
  qwen2.5-0.5b     Qwen2.5-0.5B-Instruct  Q4_K_M  ~400 MB  (smallest)
  tinyllama-1.1b   TinyLlama-1.1B-Chat    Q4_K_M  ~670 MB

Custom URL:
  scripts/download-model.sh --url <https-url> <output-filename>
EOF
        exit 0
        ;;
    *)
        echo "unknown alias '$ALIAS'. Run: scripts/download-model.sh list" >&2
        exit 1
        ;;
esac

DEST="models/$FILE"
if [ -f "$DEST" ]; then
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
