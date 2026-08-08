#!/bin/bash

# ==============================================================================
# Stock Crawler Control Script
# 支援本機與 Docker 部署模式
# ==============================================================================

# 遇到錯誤立即停止執行
set -e

export script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export built_path="./target/release"
export app_path="./bin"
export binary_name="stock_crawler"
# 開發機交叉編譯好的執行檔上傳到這裡（scp 到 /tmp），move 再從這裡搬到 script_dir。
export upload_dir="${UPLOAD_BIN_DIR:-/tmp}"
export pid_file="$script_dir/${app_path#./}/$binary_name.pid"
export stdout_log="$script_dir/${app_path#./}/nohup.out"

# SSL 憑證掛載目錄：從 .env 的 SYSTEM_SSL_CERT_FILE 反推目錄，
# 確保跟程式實際讀取憑證的路徑永遠一致（如需調整請直接修改 .env）。
env_file="$script_dir/.env"
system_ssl_cert_file_from_env=""
if [ -f "$env_file" ]; then
  system_ssl_cert_file_from_env="$(grep -m1 '^SYSTEM_SSL_CERT_FILE=' "$env_file" | cut -d '=' -f2-)"
fi
system_ssl_cert_file="${SYSTEM_SSL_CERT_FILE:-$system_ssl_cert_file_from_env}"
if [ -n "$system_ssl_cert_file" ]; then
  export docker_ssl_dir="$(dirname "$system_ssl_cert_file")"
else
  export docker_ssl_dir=""
fi

# 日誌函式
log() {
  echo "[$(date +'%Y-%m-%d %H:%M:%S')] $*"
}

# 本機模式（非 Docker）用：依目前機器的 uname -m 判斷 arm64 / armv7，
# 回傳帶架構後綴的執行檔名稱（例如 stock_crawler_arm64），
# 讓 script_dir 底下可以同時放多個架構的執行檔而不會互相覆蓋。
function resolve_binary_name() {
  local arch=""
  case "$(uname -m)" in
    aarch64) arch="arm64" ;;
    armv7l) arch="armv7" ;;
    *)
      log "不支援的本機架構: $(uname -m)（僅支援 aarch64/armv7l）"
      exit 1
      ;;
  esac
  echo "${binary_name}_${arch}"
}

function start() {
  # pid file 存在且該 pid 還活著，代表服務已經在跑，不用重複啟動。
  if [ -f "$pid_file" ] && kill -0 "$(cat "$pid_file")" 2>/dev/null; then
    log "服務已在執行中 (pid $(cat "$pid_file"))"
    return 0
  fi

  local binary_name_arch bin_path
  binary_name_arch="$(resolve_binary_name)"
  # 執行檔跟 control.sh 放在同一層（跟 docker_build 用的是同一份 stock_crawler_arm64/armv7），
  # app_path（bin/）只放 nohup.out / pid file 等執行期產生的檔案。
  bin_path="$script_dir/$binary_name_arch"

  if [ ! -f "$bin_path" ]; then
    log "找不到對應架構的執行檔: $bin_path"
    exit 1
  fi
  chmod +x "$bin_path"
  mkdir -p "$app_path"

  # `nohup` 讓程序忽略登出時的 SIGHUP；`</dev/null` 避免它嘗試讀取已經消失的終端機輸入；
  # `disown` 把它從目前 shell 的 job table 移除，雙重保險避免登出時被牽連關閉。
  nohup "$bin_path" >>"$stdout_log" 2>&1 </dev/null &
  local pid=$!
  disown "$pid" 2>/dev/null || true
  echo "$pid" > "$pid_file"
  log "$binary_name_arch 啟動成功 (pid $pid)"
}

function stop() {
  # 備份日誌
  if [ -f "$stdout_log" ]; then
    mkdir -p "$app_path/log_backup"
    mv "$stdout_log" "$app_path/log_backup/nohup.out.$(date "+%Y%m%d-%H%M%S")"
  fi

  if [ ! -f "$pid_file" ]; then
    log "找不到 pid file，服務可能未啟動"
    return 0
  fi

  local pid
  pid="$(cat "$pid_file")"
  if kill -0 "$pid" 2>/dev/null; then
    kill -SIGTERM "$pid"
    log "已送出停止訊號給 pid $pid"
    wait_for_exit "$pid"
  else
    log "pid $pid 已經不存在，可能是上次未正常關閉"
  fi
  rm -f "$pid_file"
}

# 等舊程序真的退出再返回。程式的 graceful shutdown 最久要等
# gRPC 30s + Web 30s + 背景作業 5 分鐘（見 src/main.rs），
# 這裡抓 6 分鐘上限，避免 update 在舊程序還活著時就把新的啟起來變成雙實例。
function wait_for_exit() {
  local pid="$1"
  local waited=0
  local limit="${STOP_WAIT_SECONDS:-360}"

  while kill -0 "$pid" 2>/dev/null; do
    if [ "$waited" -ge "$limit" ]; then
      log "pid $pid 已等待 ${limit}s 仍未結束，請確認後手動處理（kill -9 $pid）"
      exit 1
    fi
    sleep 2
    waited=$((waited + 2))
    # 每 30 秒回報一次，讓長時間的背景作業收尾看得出還在進行中。
    if [ $((waited % 30)) -eq 0 ]; then
      log "等待 pid $pid 結束中... (${waited}s)"
    fi
  done

  log "pid $pid 已結束（耗時 ${waited}s）"
}

# 執行檔改為在開發機交叉編譯後上傳，裝置端不再 cargo build，
# 因此 update 只做「停止 → 就位 → 啟動」。
function update() {
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
  local binary_name_arch dest_path src_path backup_name
  binary_name_arch="$(resolve_binary_name)"
  # 部署到跟 control.sh 同一層，跟 docker_build 及 start() 找執行檔的位置一致。
  dest_path="$script_dir/$binary_name_arch"

  # 來源優先序：
  #   1. UPLOAD_BIN_FILE 指定的完整路徑
  #   2. 上傳目錄（預設 /tmp）底下的同名檔案 —— 開發機交叉編譯後 scp 到這裡；
  #      不直接 scp 覆蓋執行中的檔案，否則會得到 "Text file busy"(ETXTBSY)
  #   3. 本機 cargo build 的產物（保留給仍在裝置上編譯的情境）
  if [ -n "$UPLOAD_BIN_FILE" ]; then
    src_path="$UPLOAD_BIN_FILE"
  elif [ -f "$upload_dir/$binary_name_arch" ]; then
    src_path="$upload_dir/$binary_name_arch"
  elif [ -f "$built_path/$binary_name" ]; then
    src_path="$built_path/$binary_name"
  else
    log "錯誤: 找不到可部署的執行檔"
    log "請先將交叉編譯好的 $binary_name_arch 上傳到 $upload_dir/（或用 UPLOAD_BIN_FILE 指定路徑）"
    exit 1
  fi

  backup_name="$binary_name_arch.$(date "+%Y%m%d-%H%M%S")"

  # 如果舊檔案存在則備份。先把舊檔改名移走，新檔才是建立全新的 inode，
  # 就算舊程序還沒完全結束也不會踩到 ETXTBSY。
  if [ -f "$dest_path" ]; then
    mv "$dest_path" "$script_dir/$backup_name"
    chmod -x "$script_dir/$backup_name"
  fi

  mv "$src_path" "$dest_path"
  chmod +x "$dest_path"
  log "檔案部署成功: $dest_path（來源 $src_path）"
}

# --- Docker 相關指令 ---

function docker_build() {
  # Dockerfile 會依 BuildKit 自動注入的 TARGETARCH/TARGETVARIANT
  # 從 build context 根目錄選用 stock_crawler_arm64 或 stock_crawler_armv7，
  # 所以這兩個檔案都必須先備妥在專案根目錄（用 build.bat/build.ps1 交叉編譯產出）。
  # 預設直接吃專案根目錄下的兩個檔案（從 Windows 用 build.bat/build.ps1
  # 交叉編譯後，透過 scp/rsync 放到裝置上的 $script_dir 即可）。
  local arm64_src="${DOCKER_BIN_FILE_ARM64:-$script_dir/stock_crawler_arm64}"
  local armv7_src="${DOCKER_BIN_FILE_ARMV7:-$script_dir/stock_crawler_armv7}"
  local arm64_dst="$script_dir/stock_crawler_arm64"
  local armv7_dst="$script_dir/stock_crawler_armv7"

  if [ ! -f "$arm64_src" ]; then
    log "找不到 arm64 binary: $arm64_src"
    log "可設定 DOCKER_BIN_FILE_ARM64，或先用 build.bat/build.ps1 產出對應 binary 並放到專案根目錄。"
    exit 1
  fi
  if [ ! -f "$armv7_src" ]; then
    log "找不到 armv7 binary: $armv7_src"
    log "可設定 DOCKER_BIN_FILE_ARMV7，或先用 build.bat/build.ps1 產出對應 binary 並放到專案根目錄。"
    exit 1
  fi

  [ "$arm64_src" = "$arm64_dst" ] || cp "$arm64_src" "$arm64_dst"
  [ "$armv7_src" = "$armv7_dst" ] || cp "$armv7_src" "$armv7_dst"

  log "開始建立 Docker 映像檔..."
  cd "$script_dir"
  docker build -t stock-rust-image -f Dockerfile .
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
  local docker_log_dir="${DOCKER_LOG_DIR:-$script_dir/log}"

  mkdir -p "$docker_log_dir"

  # 未設定 SYSTEM_SSL_CERT_FILE 時 docker_ssl_dir 會是空字串，
  # 此時不掛載 SSL 目錄（gRPC 會以非加密模式啟動），避免產生
  # `-v="::ro"` 這種不合法的掛載參數把 docker run 整個搞掛。
  local ssl_volume_args=()
  if [ -n "$docker_ssl_dir" ]; then
    ssl_volume_args=(-v="$docker_ssl_dir:$docker_ssl_dir:ro")
  else
    log "未設定 SYSTEM_SSL_CERT_FILE，略過掛載 SSL 憑證目錄"
  fi

  log "啟動 Docker 容器..."
  docker run --name stock-rust-container \
    -v="$docker_log_dir:/app/log:rw" \
    "${ssl_volume_args[@]}" \
    -p 9001:9001 -p 9002:9002 -t -d stock-rust-image
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
  echo "    build          - 編譯專案（裝置端自行編譯時才需要）"
  echo "    move           - 將上傳的執行檔就位（來源：UPLOAD_BIN_FILE > $upload_dir/ > target/release）"
  echo "    update         - 完整更新 (stop + move + start)"
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
