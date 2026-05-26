#!/usr/bin/env bash
set -Eeuo pipefail

# 從原始碼下載、解壓、設定並安裝 nginx 或 freenginx。
# 預設會從官方下載頁抓取最新版；設定 LATEST=0 時才會使用下方固定版本。

SOURCE_DIR="${SOURCE_DIR:-/opt/nginx/source}"
INSTALL_ROOT="${INSTALL_ROOT:-/opt/nginx}"
INSTALL_NAME="${INSTALL_NAME:-}"
FLAVOR="${FLAVOR:-freenginx}"
CHANNEL="${CHANNEL:-mainline}"
LATEST="${LATEST:-1}"
WITH_BROTLI="${WITH_BROTLI:-1}"
WITH_GEOIP="${WITH_GEOIP:-1}"
WITH_IMAGE_FILTER="${WITH_IMAGE_FILTER:-1}"
BROTLI_DIR="${BROTLI_DIR:-/opt/nginx/source/ngx_brotli}"
OPENSSL_PROVIDER="${OPENSSL_PROVIDER:-openssl}"
DRY_RUN="${DRY_RUN:-0}"
JOBS="${JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)}"

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
    "--with-stream"
    "--with-threads"
    "--with-stream_ssl_module"
    "--with-stream_ssl_preread_module"
    "--with-stream_realip_module"
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

main() {
  need_cmd tar
  need_cmd grep
  need_cmd sed
  need_cmd sort
  need_cmd awk

  if ! command -v curl >/dev/null 2>&1 && ! command -v wget >/dev/null 2>&1; then
    die "需要 curl 或 wget 其中一個下載工具"
  fi

  resolve_latest_versions
  download_sources
  build_server

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
}

main "$@"
