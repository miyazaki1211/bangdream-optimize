#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

export BANGDREAM_OPTIMIZE_HOST="${BANGDREAM_OPTIMIZE_HOST:-0.0.0.0}"
export BANGDREAM_OPTIMIZE_PORT="${BANGDREAM_OPTIMIZE_PORT:-3100}"
export BANGDREAM_OPTIMIZE_MONGODB_URI="${BANGDREAM_OPTIMIZE_MONGODB_URI:-mongodb://127.0.0.1:27017}"
export BANGDREAM_OPTIMIZE_MONGODB_DB="${BANGDREAM_OPTIMIZE_MONGODB_DB:-tsugu-bangdream-bot}"
export BANGDREAM_OPTIMIZE_GAME_DATA_ROOT="${BANGDREAM_OPTIMIZE_GAME_DATA_ROOT:-var/game-data}"
# 缓存镜像模式（方案1）：以自身 /game-data 静态路由为镜像源，实现数据变化后自动热重载
export BANGDREAM_OPTIMIZE_GAME_DATA_BASE_URL="${BANGDREAM_OPTIMIZE_GAME_DATA_BASE_URL:-http://127.0.0.1:${BANGDREAM_OPTIMIZE_PORT:-3100}/game-data}"
export BANGDREAM_OPTIMIZE_GAME_DATA_CACHE_ROOT="${BANGDREAM_OPTIMIZE_GAME_DATA_CACHE_ROOT:-var/game-data-cache}"
# 关键：显式固定同步输出目录，防止 CACHE_ROOT 劫持同步写入位置
export BANGDREAM_OPTIMIZE_GAME_DATA_SYNC_OUT="${BANGDREAM_OPTIMIZE_GAME_DATA_SYNC_OUT:-var/game-data}"
export BANGDREAM_OPTIMIZE_GAME_DATA_SYNC_ENABLED="${BANGDREAM_OPTIMIZE_GAME_DATA_SYNC_ENABLED:-1}"
export BANGDREAM_OPTIMIZE_GAME_DATA_SYNC_INTERVAL_SECONDS="${BANGDREAM_OPTIMIZE_GAME_DATA_SYNC_INTERVAL_SECONDS:-3600}"
export BANGDREAM_OPTIMIZE_TELEMETRY_JSONL="${BANGDREAM_OPTIMIZE_TELEMETRY_JSONL:-var/telemetry/internal.jsonl}"
export RUST_LOG="${RUST_LOG:-info}"

profile="${BANGDREAM_OPTIMIZE_CARGO_PROFILE:-release}"
cmd=(cargo run -p bangdream-optimize-server --bin bangdream-optimize-server)

if [[ "${profile}" == "release" ]]; then
  cmd+=(--release)
elif [[ "${profile}" != "dev" ]]; then
  echo "unsupported BANGDREAM_OPTIMIZE_CARGO_PROFILE=${profile}; use dev or release" >&2
  exit 1
fi
if (( $# > 0 )); then
  cmd+=(-- "$@")
fi

echo "server: http://${BANGDREAM_OPTIMIZE_HOST}:${BANGDREAM_OPTIMIZE_PORT}"
if [[ "${BANGDREAM_OPTIMIZE_MONGODB_URI}" == *://*@* ]]; then
  echo "mongodb: configured with credentials (db: ${BANGDREAM_OPTIMIZE_MONGODB_DB})"
else
  echo "mongodb: ${BANGDREAM_OPTIMIZE_MONGODB_URI} (db: ${BANGDREAM_OPTIMIZE_MONGODB_DB})"
fi
if [[ -n "${BANGDREAM_OPTIMIZE_WEB_ROOT:-}" ]]; then
  echo "web root: ${BANGDREAM_OPTIMIZE_WEB_ROOT}"
else
  echo "web root: disabled; run ./scripts/run-web.sh for the web UI"
fi
echo "game data: ${BANGDREAM_OPTIMIZE_GAME_DATA_ROOT}"
echo "game data mirror: ${BANGDREAM_OPTIMIZE_GAME_DATA_BASE_URL}"
echo "game data cache: ${BANGDREAM_OPTIMIZE_GAME_DATA_CACHE_ROOT}"
echo "game data sync: ${BANGDREAM_OPTIMIZE_GAME_DATA_SYNC_ENABLED}"
echo "game data sync interval: ${BANGDREAM_OPTIMIZE_GAME_DATA_SYNC_INTERVAL_SECONDS}s"
echo "telemetry: ${BANGDREAM_OPTIMIZE_TELEMETRY_JSONL}"

exec "${cmd[@]}"
