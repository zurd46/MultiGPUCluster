#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

for f in .dev-logs/*.pid; do
    [ -f "$f" ] || continue
    name=$(basename "$f" .pid)
    pid=$(cat "$f")
    if kill -0 "$pid" 2>/dev/null; then
        echo "==> stopping ${name} (pid ${pid})"
        kill "$pid"
    fi
    rm -f "$f"
done
echo "all dev services stopped"
