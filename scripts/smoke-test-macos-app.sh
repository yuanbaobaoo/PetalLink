#!/bin/bash
set -euo pipefail

APP=${1:?usage: smoke-test-macos-app.sh APP}
BIN="$APP/Contents/MacOS/PetalLink"
DATA_DIR=$(mktemp -d "${TMPDIR:-/tmp}/petallink-smoke.XXXXXX")
LOG="$DATA_DIR/launch.log"
PID=""

cleanup() {
  if [[ -n "$PID" ]] && kill -0 "$PID" 2>/dev/null; then
    kill "$PID" 2>/dev/null || true
    for _ in {1..20}; do
      kill -0 "$PID" 2>/dev/null || break
      sleep 0.1
    done
    kill -KILL "$PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

test -x "$BIN"
env PETALLINK_DATA_DIR="$DATA_DIR" "$BIN" --hidden >"$LOG" 2>&1 &
PID=$!

for _ in {1..40}; do
  if ! kill -0 "$PID" 2>/dev/null; then
    cat "$LOG"
    echo "PetalLink packaged app exited during startup" >&2
    exit 1
  fi
  grep -q '应用服务已装配' "$LOG" && break
  sleep 0.25
done

if ! grep -q '应用服务已装配' "$LOG"; then
  cat "$LOG"
  echo "PetalLink packaged app did not finish startup" >&2
  exit 1
fi

# 同一数据目录共享单实例锁；第二实例应发送 SHOW 并立即正常退出。
env PETALLINK_DATA_DIR="$DATA_DIR" "$BIN" --hidden
sleep 0.5
kill -0 "$PID"

echo "PetalLink packaged app smoke test passed: hidden launch + single instance"
