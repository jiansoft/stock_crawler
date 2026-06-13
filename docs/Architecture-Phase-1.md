# 專案架構盤點

更新日期：2026-06-13

## 專案結構

```text
stock_rust/
├─ Cargo.toml              # 單一 crate manifest，package name: stock_crawler
├─ Cargo.lock              # 鎖定依賴版本
├─ build.rs                # 產生 tonic/prost gRPC stub 到 src/interfaces/rpc/
├─ app.json                # 預設設定
├─ src/
│  ├─ main.rs              # 組裝根與服務啟動
│  ├─ core/                # 共用設定、日誌、宣告、工具
│  ├─ domain/              # 領域實體、事件、repository trait
│  ├─ app/                 # 排程、回補、事件處理、計算用例
│  ├─ infra/               # PostgreSQL、Redis、cache、crawler
│  └─ interfaces/          # gRPC、HTTP/Axum、Telegram
├─ etc/
│  ├─ proto/               # gRPC contract
│  └─ sql/                 # PostgreSQL schema / migration SQL
├─ docs/                   # 架構與重構文件
├─ scripts/                # 輔助腳本
├─ .github/workflows/      # Rust CI、CodeQL、SLSA
├─ Dockerfile              # 內容疑似屬於 Go 專案，待確認
└─ Dockerfile_live         # Rust runtime distroless image
```

目前未發現 `crates/`、`bins/`、`tests/`、`benches/`、`examples/` 目錄。

## 目前架構圖

```text
main.rs
  ├─ core
  ├─ domain
  ├─ app
  │  ├─ backfill
  │  ├─ calculation
  │  ├─ event
  │  └─ scheduler
  ├─ infra
  │  ├─ cache
  │  ├─ crawler
  │  ├─ database
  │  │  ├─ repository
  │  │  └─ table
  │  └─ nosql
  └─ interfaces
     ├─ bot
     ├─ rpc
     └─ web
```

Mermaid 版本見 [diagrams/Architecture.mmd](diagrams/Architecture.mmd)。

## 依賴方向觀察

目標依賴方向應為：

```text
interfaces -> app -> domain
infra      -> domain
core       -> 無上層依賴
main.rs    -> 組裝所有層
```

目前實際觀察：

- `domain` 主要依賴 `core` 與自身模組，方向合理。
- `infra/database/repository` 實作 `domain/*/repository.rs` trait，方向合理。
- `app` 仍直接 new 具體 PostgreSQL repository、呼叫 Redis/cache/crawler/bot，應用層與基礎設施耦合偏高。
- `infra/database/repository/quote.rs` 直接呼叫 Redis cache invalidation，資料庫 repository 與 cache 有橫切耦合。
- `infra/crawler/yahoo/price/cache.rs` 呼叫 `interfaces::bot::telegram`，形成 infra 到 interfaces 的反向依賴風險。
- `infra/database/table/quote/daily_quote/mod.rs` 測試區引用 `app::backfill::acl`，雖位於測試但仍使邊界模糊。

## 建置與產碼

- `build.rs` 使用 `protoc-bin-vendored` 與 `tonic-prost-build`。
- proto 來源：`etc/proto/basic.proto`、`control.proto`、`manual_backfill.proto`、`stock.proto`。
- 產出位置：`src/interfaces/rpc/`。
- `src/interfaces/rpc/basic.rs`、`control.rs`、`manual_backfill.rs`、`stock.rs` 為產碼，不應手動修改。

## CI/CD

- Rustfmt：`cargo fmt --all -- --check`
- Check/Clippy：`cargo check --verbose`、`cargo clippy -- -D warnings`
- Build：`cargo build --verbose`
- Test：啟動 PostgreSQL 與 Redis，依 `.github/workflows/rust.yml` 執行 SQL bootstrap，最後執行 `cargo test --release -- --nocapture --test-threads=1`
- CodeQL：Rust 與 GitHub Actions
- Dependabot：cargo 與 GitHub Actions weekly

## Docker

- `Dockerfile_live` 是目前 Rust runtime 映像，使用 distroless static Debian 13 nonroot。
- 根目錄 `Dockerfile` 內容包含 `golang`、`go.mod`、`cleanupdb`，與本 Rust 專案不一致。是否仍由外部流程使用：待確認（To Be Verified）。

