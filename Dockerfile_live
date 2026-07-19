# 第一階段：把 arm64 / armv7 兩個平台的已編譯 binary 都收進來，
# 再依照 BuildKit 自動注入的 TARGETARCH / TARGETVARIANT 選出符合目前建置目標的那一個。
#
# - linux/arm64  → TARGETARCH=arm64（TARGETVARIANT 通常為空）
# - linux/arm/v7 → TARGETARCH=arm，TARGETVARIANT=v7（Raspberry Pi 3 的 armv7l）
#
# 不需要手動指定 --build-arg：不論是在裝置上直接 `docker build`，
# 還是用 `docker buildx build --platform linux/arm64,linux/arm/v7` 跨平台建置，
# BuildKit 都會依目標平台自動帶入這兩個值。
# 這個階段需要 shell 才能做條件判斷，所以不能用 `scratch`，
# 但最終產物只有 `/stock_crawler` 這個檔案會被下一階段複製出去，不會影響最終 image 大小。
FROM alpine:3 AS binary
ARG TARGETARCH
ARG TARGETVARIANT
COPY stock_crawler_arm64 /src/stock_crawler_arm64
COPY stock_crawler_armv7 /src/stock_crawler_armv7
RUN set -eu; \
    case "${TARGETARCH}-${TARGETVARIANT}" in \
      arm64-*) cp /src/stock_crawler_arm64 /stock_crawler ;; \
      arm-v7) cp /src/stock_crawler_armv7 /stock_crawler ;; \
      *) echo "不支援的平台: TARGETARCH=${TARGETARCH} TARGETVARIANT=${TARGETVARIANT}" >&2; exit 1 ;; \
    esac; \
    chmod 755 /stock_crawler

# distroless static 已內建 CA 憑證（/etc/ssl/certs/ca-certificates.crt）與
# 完整 tzdata（/usr/share/zoneinfo，含 Asia/Taipei），不需再從 debian 複製。
FROM gcr.io/distroless/static-debian13:nonroot

ENV TZ=Asia/Taipei
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
WORKDIR /app

COPY --from=binary --chown=65532:65532 --chmod=755 /stock_crawler /app/stock_crawler
COPY --chown=65532:65532 ./.env /app/.env
COPY --chown=65532:65532 ./app.json /app/app.json

EXPOSE 9001

USER 65532:65532

ENTRYPOINT ["/app/stock_crawler"]
