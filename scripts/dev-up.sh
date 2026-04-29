#!/usr/bin/env bash
# Start the full backend locally without Docker for development.
# Logs go to .dev-logs/, PIDs to .dev-logs/*.pid
# Stop with: scripts/dev-down.sh
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p .dev-logs

# Make sure cargo is on PATH (rustup install path)
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

build_first() {
    echo "==> building all binaries (first run can take a few minutes)..."
    cargo build \
        -p gpucluster-coordinator \
        -p gpucluster-gateway \
        -p gpucluster-mgmt-backend \
        -p gpucluster-openai-api
}

start() {
    local name="$1"; shift
    if [ -f ".dev-logs/${name}.pid" ] && kill -0 "$(cat .dev-logs/${name}.pid)" 2>/dev/null; then
        echo "==> ${name} already running (pid $(cat .dev-logs/${name}.pid))"
        return
    fi
    echo "==> starting ${name}"
    nohup "$@" > ".dev-logs/${name}.log" 2>&1 &
    echo $! > ".dev-logs/${name}.pid"
}

build_first

start coordinator ./target/debug/gpucluster-coordinator
start gateway     ./target/debug/gpucluster-gateway
start mgmt        ./target/debug/gpucluster-mgmt
start openai-api  ./target/debug/gpucluster-openai-api

sleep 1
echo
echo "Services up. Health checks:"
for url in \
    "http://localhost:7001/health   coordinator" \
    "http://localhost:8443/health   gateway" \
    "http://localhost:7100/health   mgmt-backend" \
    "http://localhost:7200/health   openai-api"
do
    set -- $url
    code=$(curl -s -o /dev/null -w "%{http_code}" "$1" || echo "ERR")
    printf "  %-15s %s  -> %s\n" "$2" "$1" "$code"
done
echo
echo "Logs:    tail -f .dev-logs/*.log"
echo "Stop:    scripts/dev-down.sh"
