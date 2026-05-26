#!/usr/bin/env bash
set -Eeuo pipefail

# 從原始碼下載、解壓、設定並安裝 F5 維護的官方 nginx 或 freenginx。
# 預設使用 F5 官方 nginx 來源並抓取最新版；設定 LATEST=0 時才會使用下方固定版本。

SOURCE_DIR="${SOURCE_DIR:-/opt/nginx/source}"
INSTALL_ROOT="${INSTALL_ROOT:-/opt/nginx}"
INSTALL_NAME="${INSTALL_NAME:-}"
FLAVOR="${FLAVOR:-nginx}"
CHANNEL="${CHANNEL:-mainline}"
LATEST="${LATEST:-1}"
WITH_BROTLI="${WITH_BROTLI:-1}"
WITH_GEOIP="${WITH_GEOIP:-1}"
WITH_IMAGE_FILTER="${WITH_IMAGE_FILTER:-1}"
BROTLI_DIR="${BROTLI_DIR:-/opt/nginx/source/ngx_brotli}"
OPENSSL_PROVIDER="${OPENSSL_PROVIDER:-openssl}"
DRY_RUN="${DRY_RUN:-0}"
JOBS="${JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)}"
RESTART_AFTER_INSTALL="${RESTART_AFTER_INSTALL:-1}"
CURRENT_LINK="${CURRENT_LINK:-${INSTALL_ROOT}/current}"
MODULES_LINK="${MODULES_LINK:-${INSTALL_ROOT}/modules}"
NGINX_PID_FILE="${NGINX_PID_FILE:-${INSTALL_ROOT}/run/nginx.pid}"
STOP_TIMEOUT_SECONDS="${STOP_TIMEOUT_SECONDS:-30}"

NGINX_VERSION="${NGINX_VERSION:-1.31.0}"
FREENGINX_VERSION="${FREENGINX_VERSION:-1.31.0}"
ZLIB_VERSION="${ZLIB_VERSION:-1.3.2}"
OPENSSL_VERSION="${OPENSSL_VERSION:-4.0.0}"
PCRE2_VERSION="${PCRE2_VERSION:-10.47}"
LIBRESSL_VERSION="${LIBRESSL_VERSION:-4.2.1}"

log() {
  printf '[%s] %s\n' "$(date '+%F %T')" "$*"
}

die() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "找不到必要指令：$1"
}

run() {
  log "+ $*"
  if [ "$DRY_RUN" != "1" ]; then
    "$@"
  fi
}

fetch_text() {
  local url="$1"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url"
  else
    wget -qO- "$url"
  fi
}

latest_from_url() {
  local url="$1"
  local pattern="$2"

  # 先使用 grep 過濾目標行，再使用 grep 提取純版本號（數字加小數點的組合）
  fetch_text "$url" \
    | grep -Eo "$pattern" \
    | grep -Eo '[0-9]+(\.[0-9]+)+' \
    | sort -V \
    | tail -n 1
}

latest_server_from_url() {
  local url="$1"
  local pattern="$2"
  local channel="$3"

  # 先使用 grep 過濾目標行，再使用 grep 提取主版本號（包含三個數位及兩個點）
  fetch_text "$url" \
    | grep -Eo "$pattern" \
    | grep -Eo '[0-9]+\.[0-9]+\.[0-9]+' \
    | awk -F. -v channel="$channel" '
        channel == "stable" && ($2 % 2) == 0 { print }
        channel == "mainline" && ($2 % 2) == 1 { print }
      ' \
    | sort -V \
    | tail -n 1
}

require_version() {
  local name="$1"
  local value="$2"

  [ -n "$value" ] || die "無法解析最新版本：$name"
}

resolve_latest_versions() {
  [ "$LATEST" = "1" ] || return 0

  log "檢查官方下載頁上的最新版本"

  case "$CHANNEL" in
    mainline|stable) ;;
    *) die "CHANNEL 只能是 mainline 或 stable，目前是：$CHANNEL" ;;
  esac

  if [ "$FLAVOR" = "nginx" ]; then
    NGINX_VERSION="$(latest_server_from_url 'https://nginx.org/download/' 'nginx-[0-9]+\.[0-9]+\.[0-9]+\.tar\.gz' "$CHANNEL")"
    require_version "nginx" "$NGINX_VERSION"
  elif [ "$FLAVOR" = "freenginx" ]; then
    FREENGINX_VERSION="$(latest_server_from_url 'https://freenginx.org/download/' 'freenginx-[0-9]+\.[0-9]+\.[0-9]+\.tar\.gz' "$CHANNEL")"
    require_version "freenginx" "$FREENGINX_VERSION"
  else
    die "FLAVOR 只能是 nginx 或 freenginx，目前是：$FLAVOR"
  fi

  ZLIB_VERSION="$(latest_from_url 'https://zlib.net/' 'zlib-[0-9]+\.[0-9]+\.[0-9]+\.tar\.gz')"
  PCRE2_VERSION="$(latest_from_url 'https://github.com/PCRE2Project/pcre2/releases' 'pcre2-[0-9]+\.[0-9]+')"
  require_version "zlib" "$ZLIB_VERSION"
  require_version "PCRE2" "$PCRE2_VERSION"

  case "$OPENSSL_PROVIDER" in
    openssl)
      OPENSSL_VERSION="$(latest_from_url 'https://openssl-library.org/source/' 'openssl-[0-9]+\.[0-9]+\.[0-9]+\.tar\.gz')"
      require_version "OpenSSL" "$OPENSSL_VERSION"
      ;;
    libressl)
      LIBRESSL_VERSION="$(latest_from_url 'https://ftp.openbsd.org/pub/OpenBSD/LibreSSL/' 'libressl-[0-9]+\.[0-9]+\.[0-9]+\.tar\.gz')"
      require_version "LibreSSL" "$LIBRESSL_VERSION"
      ;;
    *)
      die "OPENSSL_PROVIDER 只能是 openssl 或 libressl，目前是：$OPENSSL_PROVIDER"
      ;;
  esac
}

download() {
  local url="$1"
  local file="$2"

  if [ -f "$file" ]; then
    log "已存在，略過下載：$file"
    return 0
  fi

  if command -v curl >/dev/null 2>&1; then
    run curl -fL --retry 3 -o "$file" "$url"
  else
    run wget -O "$file" "$url"
  fi
}

extract_tar() {
  local file="$1"
  local dir="$2"

  if [ -d "$dir" ]; then
    log "已存在，略過解壓：$dir"
    return 0
  fi

  case "$file" in
    *.tar.gz|*.tgz) run tar -xzf "$file" ;;
    *.tar.bz2) run tar -xjf "$file" ;;
    *) die "不支援的壓縮檔格式：$file" ;;
  esac
}

download_sources() {
  run mkdir -p "$SOURCE_DIR"
  cd "$SOURCE_DIR"

  if [ "$FLAVOR" = "nginx" ]; then
    download "https://nginx.org/download/nginx-${NGINX_VERSION}.tar.gz" "nginx-${NGINX_VERSION}.tar.gz"
    extract_tar "nginx-${NGINX_VERSION}.tar.gz" "nginx-${NGINX_VERSION}"
    SERVER_DIR="nginx-${NGINX_VERSION}"
    SERVER_VERSION="$NGINX_VERSION"
  else
    download "https://freenginx.org/download/freenginx-${FREENGINX_VERSION}.tar.gz" "freenginx-${FREENGINX_VERSION}.tar.gz"
    extract_tar "freenginx-${FREENGINX_VERSION}.tar.gz" "freenginx-${FREENGINX_VERSION}"
    SERVER_DIR="freenginx-${FREENGINX_VERSION}"
    SERVER_VERSION="$FREENGINX_VERSION"
  fi

  download "https://zlib.net/zlib-${ZLIB_VERSION}.tar.gz" "zlib-${ZLIB_VERSION}.tar.gz"
  extract_tar "zlib-${ZLIB_VERSION}.tar.gz" "zlib-${ZLIB_VERSION}"

  download "https://github.com/PCRE2Project/pcre2/releases/download/pcre2-${PCRE2_VERSION}/pcre2-${PCRE2_VERSION}.tar.bz2" "pcre2-${PCRE2_VERSION}.tar.bz2"
  extract_tar "pcre2-${PCRE2_VERSION}.tar.bz2" "pcre2-${PCRE2_VERSION}"

  case "$OPENSSL_PROVIDER" in
    openssl)
      download "https://github.com/openssl/openssl/releases/download/openssl-${OPENSSL_VERSION}/openssl-${OPENSSL_VERSION}.tar.gz" "openssl-${OPENSSL_VERSION}.tar.gz"
      extract_tar "openssl-${OPENSSL_VERSION}.tar.gz" "openssl-${OPENSSL_VERSION}"
      TLS_DIR="../openssl-${OPENSSL_VERSION}"
      ;;
    libressl)
      download "https://ftp.openbsd.org/pub/OpenBSD/LibreSSL/libressl-${LIBRESSL_VERSION}.tar.gz" "libressl-${LIBRESSL_VERSION}.tar.gz"
      extract_tar "libressl-${LIBRESSL_VERSION}.tar.gz" "libressl-${LIBRESSL_VERSION}"
      TLS_DIR="../libressl-${LIBRESSL_VERSION}"
      ;;
  esac

  if [ "$WITH_BROTLI" = "1" ] && [ ! -d "$BROTLI_DIR" ]; then
    need_cmd git
    run git clone --recursive https://github.com/google/ngx_brotli.git "$BROTLI_DIR"
  fi
}

build_server() {
  local install_dir

  cd "$SOURCE_DIR/$SERVER_DIR"
  install_dir="${INSTALL_ROOT}/${INSTALL_NAME:-$SERVER_VERSION}"
  SERVER_INSTALL_DIR="$install_dir"

  local configure_args=(
    "--prefix=${install_dir}"
    "--sbin-path=${install_dir}/nginx"
    "--with-openssl=${TLS_DIR}"
    "--with-zlib=../zlib-${ZLIB_VERSION}"
    "--with-pcre=../pcre2-${PCRE2_VERSION}"
    "--with-compat"
    "--user=nginx"
    "--group=nginx"
    "--pid-path=${INSTALL_ROOT}/run/nginx.pid"
    "--conf-path=${INSTALL_ROOT}/nginx.conf"
    "--lock-path=${INSTALL_ROOT}/lock/nginx.lock"
    "--error-log-path=${INSTALL_ROOT}/log/error.log"
    "--http-log-path=${INSTALL_ROOT}/log/access.log"
    "--http-client-body-temp-path=${INSTALL_ROOT}/tmp/nginx/client/"
    "--http-proxy-temp-path=${INSTALL_ROOT}/tmp/nginx/proxy/"
    "--http-fastcgi-temp-path=${INSTALL_ROOT}/tmp/nginx/fcgi/"
    "--with-pcre-jit"
    "--with-http_realip_module"
    "--with-http_addition_module"
    "--with-http_ssl_module"
    "--with-http_v2_module"
    "--with-http_v3_module"
    "--with-http_sub_module"
    "--with-http_stub_status_module"
    "--with-http_mp4_module"
    "--with-http_flv_module"
    "--with-http_gzip_static_module"
    "--with-http_gunzip_module"
    "--with-http_auth_request_module"
    "--with-http_secure_link_module"
    "--with-http_slice_module"
    "--with-stream"
    "--with-threads"
    "--with-stream_ssl_module"
    "--with-stream_ssl_preread_module"
    "--with-stream_realip_module"
    "--with-cc-opt=-O3 -fstack-protector-strong -Wformat -Werror=format-security -D_FORTIFY_SOURCE=2 -fPIC"
    "--with-ld-opt=-Wl,-z,relro -Wl,-z,now -Wl,-Bsymbolic-functions"
    "--build=${FLAVOR}-${SERVER_VERSION}-${OPENSSL_PROVIDER}"
  )

  if [ "$WITH_BROTLI" = "1" ]; then
    configure_args+=("--add-dynamic-module=${BROTLI_DIR}")
  fi

  if [ "$WITH_IMAGE_FILTER" = "1" ]; then
    configure_args+=("--with-http_image_filter_module")
  fi

  if [ "$WITH_GEOIP" = "1" ]; then
    configure_args+=("--with-http_geoip_module")
  fi

  run mkdir -p "${INSTALL_ROOT}/run" "${INSTALL_ROOT}/lock" "${INSTALL_ROOT}/log" "${INSTALL_ROOT}/tmp/nginx/client" "${INSTALL_ROOT}/tmp/nginx/proxy" "${INSTALL_ROOT}/tmp/nginx/fcgi"
  run ./configure "${configure_args[@]}"
  run make -j "$JOBS"
  run make install
}

wait_for_nginx_stop() {
  local pid="$1"
  local elapsed=0

  # 等待舊程序真正離開，避免新版啟動時搶不到 listen port 或 pid 檔。
  while kill -0 "$pid" >/dev/null 2>&1; do
    if [ "$elapsed" -ge "$STOP_TIMEOUT_SECONDS" ]; then
      die "等待 nginx 停止逾時（PID: $pid，逾時秒數：$STOP_TIMEOUT_SECONDS）"
    fi

    sleep 1
    elapsed=$((elapsed + 1))
  done
}

stop_running_nginx() {
  local pid

  if [ ! -s "$NGINX_PID_FILE" ]; then
    log "找不到 nginx pid 檔，略過停止步驟：$NGINX_PID_FILE"
    return 0
  fi

  pid="$(cat "$NGINX_PID_FILE")"
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    log "pid 檔存在但程序未執行，略過停止步驟：$NGINX_PID_FILE (PID: $pid)"
    return 0
  fi

  # 使用 QUIT 讓目前 nginx 優雅停止，確定舊 master 結束後才會啟動新版。
  run kill -QUIT "$pid"
  if [ "$DRY_RUN" != "1" ]; then
    wait_for_nginx_stop "$pid"
  fi
}

require_restart_privileges() {
  if [ "$DRY_RUN" = "1" ]; then
    return 0
  fi

  # 重啟 nginx 需要寫入 /opt/nginx/log、送 signal 給舊 master，通常也要綁定 80/443。
  if [ "$(id -u)" != "0" ]; then
    die "RESTART_AFTER_INSTALL=1 需要 root 權限；請用 sudo 執行，或設定 RESTART_AFTER_INSTALL=0 只建置安裝"
  fi
}

switch_version_links() {
  # current 指向最新安裝目錄，方便外部 service 或人工檢查固定使用同一路徑。
  run ln -sfnT "$SERVER_INSTALL_DIR" "$CURRENT_LINK"

  if [ ! -d "${SERVER_INSTALL_DIR}/modules" ]; then
    log "新版本沒有 modules 目錄，略過 modules 切換：${SERVER_INSTALL_DIR}/modules"
    return 0
  fi

  if [ -L "$MODULES_LINK" ] || [ ! -e "$MODULES_LINK" ]; then
    # 若 modules 目前是 symlink 或不存在，就直接切到新版本的 dynamic modules。
    run ln -sfnT "${SERVER_INSTALL_DIR}/modules" "$MODULES_LINK"
    return 0
  fi

  if [ -d "$MODULES_LINK" ]; then
    if compgen -G "${SERVER_INSTALL_DIR}/modules/*.so" >/dev/null; then
      # 若 modules 是既有實體目錄，就覆蓋複製新版 .so，避免刪除使用者自放的模組。
      run cp -f "${SERVER_INSTALL_DIR}"/modules/*.so "${MODULES_LINK}/"
    else
      log "新版本 modules 目錄沒有 .so 檔，略過 modules 複製"
    fi
    return 0
  fi

  die "modules 路徑已存在但不是目錄或 symlink：$MODULES_LINK"
}

switch_to_new_server() {
  local new_binary="${SERVER_INSTALL_DIR}/nginx"

  [ "$RESTART_AFTER_INSTALL" = "1" ] || {
    log "RESTART_AFTER_INSTALL=0，略過切換與重啟"
    return 0
  }

  [ -x "$new_binary" ] || die "找不到新編譯的 nginx 執行檔：$new_binary"

  require_restart_privileges
  switch_version_links

  # 先用新 binary 驗證設定檔，避免停掉舊服務後才發現新版無法啟動。
  run "$new_binary" -t -c "${INSTALL_ROOT}/nginx.conf" -p "${INSTALL_ROOT}/"

  stop_running_nginx

  # 停止舊程序後，明確用剛編譯出的 binary 啟動新版 nginx。
  run "$new_binary" -c "${INSTALL_ROOT}/nginx.conf" -p "${INSTALL_ROOT}/"
}

main() {
  need_cmd tar
  need_cmd grep
  need_cmd sed
  need_cmd sort
  need_cmd awk
  need_cmd kill
  need_cmd sleep
  need_cmd ln
  need_cmd id
  need_cmd cp

  if ! command -v curl >/dev/null 2>&1 && ! command -v wget >/dev/null 2>&1; then
    die "需要 curl 或 wget 其中一個下載工具"
  fi

  resolve_latest_versions
  download_sources
  build_server
  switch_to_new_server

  log "已成功安裝以下版本套件："
  if [ "$FLAVOR" = "nginx" ]; then
    log "  - Nginx: ${NGINX_VERSION}"
  else
    log "  - freenginx: ${FREENGINX_VERSION}"
  fi
  log "  - zlib: ${ZLIB_VERSION}"
  log "  - PCRE2: ${PCRE2_VERSION}"
  if [ "$OPENSSL_PROVIDER" = "openssl" ]; then
    log "  - OpenSSL: ${OPENSSL_VERSION}"
  else
    log "  - LibreSSL: ${LIBRESSL_VERSION}"
  fi

  log "完成：${INSTALL_ROOT}/${INSTALL_NAME:-$SERVER_VERSION}/nginx"
  if [ "$RESTART_AFTER_INSTALL" = "1" ]; then
    log "已切換 current：${CURRENT_LINK} -> ${SERVER_INSTALL_DIR}"
    log "已更新 modules：${MODULES_LINK}"
    log "已使用新版 nginx 啟動服務"
  fi
}

main "$@"
