@echo off
setlocal

cd /d "%~dp0\.."

set BANGDREAM_OPTIMIZE_HOST=127.0.0.1
set BANGDREAM_OPTIMIZE_PORT=3100
set BANGDREAM_OPTIMIZE_MONGODB_URI=mongodb://127.0.0.1:27017/
set BANGDREAM_OPTIMIZE_MONGODB_DB=tsugu-bangdream-bot
set BANGDREAM_OPTIMIZE_GAME_DATA_ROOT=var/game-data
rem 缓存镜像模式（方案1）：以自身 /game-data 静态路由为镜像源，实现数据变化后自动热重载
set BANGDREAM_OPTIMIZE_GAME_DATA_BASE_URL=http://127.0.0.1:%BANGDREAM_OPTIMIZE_PORT%/game-data
set BANGDREAM_OPTIMIZE_GAME_DATA_CACHE_ROOT=var/game-data-cache
rem 关键：显式固定同步输出目录，防止 CACHE_ROOT 劫持同步写入位置
set BANGDREAM_OPTIMIZE_GAME_DATA_SYNC_OUT=var/game-data
set BANGDREAM_OPTIMIZE_GAME_DATA_SYNC_ENABLED=1
set BANGDREAM_OPTIMIZE_GAME_DATA_SYNC_INTERVAL_SECONDS=3600
set BANGDREAM_OPTIMIZE_TELEMETRY_JSONL=var/telemetry/internal.jsonl
set RUST_LOG=info

echo server: http://%BANGDREAM_OPTIMIZE_HOST%:%BANGDREAM_OPTIMIZE_PORT%
echo mongodb: %BANGDREAM_OPTIMIZE_MONGODB_URI% (db: %BANGDREAM_OPTIMIZE_MONGODB_DB%)
echo game data: %BANGDREAM_OPTIMIZE_GAME_DATA_ROOT%
echo game data mirror: %BANGDREAM_OPTIMIZE_GAME_DATA_BASE_URL%
echo game data cache: %BANGDREAM_OPTIMIZE_GAME_DATA_CACHE_ROOT%
echo game data sync: %BANGDREAM_OPTIMIZE_GAME_DATA_SYNC_ENABLED%
echo game data sync interval: %BANGDREAM_OPTIMIZE_GAME_DATA_SYNC_INTERVAL_SECONDS%s
echo telemetry: %BANGDREAM_OPTIMIZE_TELEMETRY_JSONL%
echo log level: %RUST_LOG%
echo.

cargo run -p bangdream-optimize-server --bin bangdream-optimize-server --release -- %*

endlocal
