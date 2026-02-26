#!/bin/bash

# ==============================================================================
# Stock Crawler Control Script
# 支援本機與 Docker 部署模式
# ==============================================================================

# 遇到錯誤立即停止執行
set -e

export built_path="./target/release"
export app_path="./bin"
export binary_name="stock_crawler"

# 日誌函式
log() {
  echo "[$(date +'%Y-%m-%d %H:%M:%S')] $*"
}

function start() {
  # 使用 || true 避免 pidof 在找不到進程時導致 set -e 退出腳本
  pid=$(pidof "$binary_name" || true)

  if [ -z "$pid" ]; then
    mkdir -p "$app_path"
    cd "$app_path"
    nohup ./"$binary_name" > nohup.out 2>&1 &
    log "$binary_name 啟動成功"
  else
    log "$binary_name 已經在運行中 (PID: $pid)"
  fi
}

function stop() {
  # 備份日誌
  if [ -f "$app_path/nohup.out" ]; then
    mkdir -p "$app_path/log_backup"
    mv "$app_path/nohup.out" "$app_path/log_backup/nohup.out.$(date "+%Y%m%d-%H%M%S")"
  fi

  pid=$(pidof "$binary_name" || true)

  if [ -n "$pid" ]; then
    kill -SIGTERM "$pid"
    log "$binary_name 已停止"
  else
    log "找不到運行中的 $binary_name"
  fi
}

function update() {
  build
  sleep 1
  stop
  sleep 1
  move
  sleep 1
  start
}

function restart() {
  stop
  sleep 1
  start
}

function build() {
  log "開始編譯 Release 版本..."
  cargo update
  cargo build --release
  log "編譯成功"
}

function move() {
  if [ -f "$built_path/$binary_name" ]; then
    mkdir -p "$app_path"
    backup_name="$binary_name.$(date "+%Y%m%d-%H%M%S")"
    
    # 如果舊檔案存在則備份
    if [ -f "$app_path/$binary_name" ]; then
      mv "$app_path/$binary_name" "$app_path/$backup_name"
      chmod -x "$app_path/$backup_name"
    fi

    mv "$built_path/$binary_name" "$app_path/$binary_name"
    chmod +x "$app_path/$binary_name"
    log "檔案部署成功: $app_path/$binary_name"
  else
    log "錯誤: 找不到編譯後的檔案 $built_path/$binary_name"
    exit 1
  fi
}

# --- Docker 相關指令 ---

function docker_build() {
  log "開始建立 Docker 映像檔..."
  docker build -t stock-rust-image -f Dockerfile_live .
  log "清理過期的 Docker 資源..."
  docker system prune -f
}

function docker_stop() {
  log "停止並移除 Docker 容器..."
  # -f 會強制停止運行中的容器並移除，且不會因為容器不存在而報錯退出
  docker rm -f stock-rust-container 2>/dev/null || true
  docker ps -a | grep stock-rust-container || true
}

function docker_start() {
  log "啟動 Docker 容器..."
  docker run --name stock-rust-container \
    -v=/opt/stock_crawler/log:/app/log:rw \
    -v=/opt/nginx/ssl/jiansoft.mooo.com:/opt/nginx/ssl/jiansoft.mooo.com \
    -p 9001:9001 -t -d stock-rust-image
  docker ps
}

function docker_restart() {
  docker_stop
  sleep 1
  docker_start
}

function docker_update() {
  # 只有在 build 成功時才執行 restart
  docker_build
  docker_restart
}

function help() {
  echo "使用方法: $0 {指令}"
  echo "可用指令:"
  echo "  [本機模式]"
  echo "    start          - 啟動服務"
  echo "    stop           - 停止服務"
  echo "    restart        - 重啟服務"
  echo "    build          - 編譯專案"
  echo "    move           - 部署編譯後的檔案"
  echo "    update         - 完整更新 (build + stop + move + start)"
  echo ""
  echo "  [Docker 模式]"
  echo "    docker_start   - 啟動容器"
  echo "    docker_stop    - 停止並移除容器"
  echo "    docker_restart - 重啟容器"
  echo "    docker_build   - 建立映像檔"
  echo "    docker_update  - 完整更新映像檔並重啟容器"
}

# --- 參數解析 ---

case "$1" in
  start|stop|restart|update|move|build|docker_build|docker_stop|docker_start|docker_restart|docker_update)
    "$1"
    ;;
  help|--help|-h)
    help
    ;;
  *)
    help
    exit 1
    ;;
esac
