# Architecture

> Last updated: 2026-06-26

## Overview

`stock_crawler` 是一個台灣股市資料爬蟲系統，以 **DDD（領域驅動設計）五層架構**實作，負責：

- 從 14 個以上的台股資料來源（TWSE、TPEX、GoodInfo、Yahoo 等）定期爬取資料。
- 將爬取資料存入 PostgreSQL，並透過 Redis 提供即時快取。
- 透過 Telegram bot 發送到期除息、除權等事件通知。
- 提供 gRPC API 與 REST HTTP API 供外部系統查詢。

**Live Demo**: https://jiansoft.mooo.com/stock/revenues

---

## 分層架構

```
┌─────────────────────────────────────────────────┐
│                  interfaces                      │  ← gRPC server/client、Axum Web、Telegram bot
├─────────────────────────────────────────────────┤
│                     app                         │  ← scheduler、Use Case 協調、ACL Mapper
├─────────────────────────────────────────────────┤
│      infra              │       core             │  ← DB/Redis/Cache、爬蟲  │  config/logging/util
├─────────────────────────┴───────────────────────┤
│                    domain                       │  ← 實體、值物件、Repository trait
└─────────────────────────────────────────────────┘
```

**依賴方向**（單向，內層不知道外層存在）：

```
interfaces → app → infra → domain
                 → core
```

---

## 模組說明

### `domain/` — 領域模型層

純 Rust struct / trait，零外部框架依賴。每個領域包含三個檔案：

| 檔案 | 職責 |
|------|------|
| `entity.rs` | 領域實體或值物件定義 |
| `repository.rs` | 儲存庫抽象 trait（`async_trait`） |
| `mod.rs` | 對外 re-export |

#### 領域目錄

| 領域 | 路徑 | 業務語義 |
|------|------|---------|
| `config` | `domain/config/` | 系統鍵值設定（key-value store） |
| `dividend` | `domain/dividend/` | 股息發放日程（除息日、現金股利、股票股利） |
| `financial` | `domain/financial/` | 季度財務報表（EPS、營收、淨值） |
| `market_index` | `domain/market_index/` | 市場指數（TAIEX 加權指數、各類指數） |
| `money_flow` | `domain/money_flow/` | 大盤與帳戶市值總覽（買超/賣超資金流向） |
| `portfolio` | `domain/portfolio/` | 持股明細（持有股數、成本、損益） |
| `quote` | `domain/quote/` | 每日個股報價（開高低收、成交量） |
| `registry` | `domain/registry/` | 證券登錄（StockSymbol 值物件、股票基本資料） |
| `trace` | `domain/trace/` | 個股價格追蹤警示（floor / ceiling 區間） |
| `yield_rank` | `domain/yield_rank/` | 殖利率排行（依配息率排序的選股指標） |
| `events` | `domain/events.rs` | 跨領域 DomainEvent 列舉（供 registry / trace 使用） |

---

### `core/` — 基礎設施核心層

不含業務邏輯的通用工具，可被所有層引用。

| 子模組 | 路徑 | 職責 |
|--------|------|------|
| `config` | `core/config.rs` | 讀取 `app.json` + `.env`，提供全域 `SETTINGS` 單例 |
| `declare` | `core/declare.rs` | 全域列舉（`StockExchange` 等）與通用常數 |
| `logging` | `core/logging/` | 非同步輪轉日誌（`LOGGER`）、`FileLogLayer`（tracing bridge）、Seq 轉發 |
| `util` | `core/util/` | datetime、HTTP client、text 解析、diagnostics（malloc 調校） |

---

### `infra/` — 基礎設施實作層

實作 `domain` 定義的 Repository trait，並提供資料來源介接。

| 子模組 | 路徑 | 職責 |
|--------|------|------|
| `database` | `infra/database/` | SQLx + PostgreSQL 連線池（`PgPool`）、`table/` ORM struct、`repository/` 實作 |
| `nosql` | `infra/nosql/` | deadpool-redis 連線池封裝，`CLIENT` 單例，`RedisError`（thiserror） |
| `cache` | `infra/cache/` | 全域記憶體快取 `SHARE`（股票清單 + 即時快照），分為 loader / query / snapshot 三責 |
| `crawler` | `infra/crawler/` | 14 個以上資料來源的 HTTP 爬蟲，`CrawlerError`（thiserror） |

#### 爬蟲資料來源

| 來源 | 子目錄 | 資料類型 |
|------|--------|---------|
| TWSE | `crawler/twse/` | 上市股票報價、ISIN、財務 |
| TPEX | `crawler/tpex/` | 上櫃股票報價、財務 |
| TAIFEX | `crawler/taifex/` | 期貨資料 |
| GoodInfo | `crawler/goodinfo/` | 殖利率、財務分析 |
| Histock | `crawler/histock/` | 歷史報價補全 |
| CMoney | `crawler/cmoney/` | 籌碼分析 |
| MoneyDJ | `crawler/moneydj/` | 年度獲利資料 |
| Yahoo Finance | `crawler/yahoo/` | 即時報價補充 |
| Fugle | `crawler/fugle/` | 即時行情 |
| FBS | `crawler/fbs/` | 年度財務摘要 |
| MOPS | `crawler/mops/` | 公開資訊觀測站（月營收、重大訊息） |
| Cnyes | `crawler/cnyes/` | 鉅亨網資料 |
| Megatime | `crawler/megatime/` | 時報資訊 |
| Wespai / WinVest / Nstock / Yuanta | 各自子目錄 | 輔助資料來源 |

---

### `app/` — 應用服務層

協調領域操作與跨子系統流程，不含 UI 或傳輸細節。

| 子模組 | 路徑 | 職責 |
|--------|------|------|
| `scheduler` | `app/scheduler.rs` | Tokio cron 排程定義（UTC 時區），綁定所有定時任務 |
| `backfill` | `app/backfill/` | 歷史資料補填 Use Case（acl/、calculation/ 子模組） |
| `event` | `app/event/` | 除息提醒、除權提醒、公開申購提醒等事件 handler |
| `calculation` | `app/calculation/` | 估算股價、殖利率計算等業務邏輯 |
| `manual_backfill` | `app/manual_backfill.rs` | 手動觸發的一次性補填任務 |

#### ACL（Anti-Corruption Layer）

位於 `app/backfill/acl/`，負責將爬蟲 DTO 轉換為領域 Command，隔離外部資料格式變化對領域的影響。

```
CrawlerDto → [ACL Mapper] → SaveXxxCommand → domain Repository
```

---

### `interfaces/` — 外部介面層

處理所有輸入/輸出協定，不含業務邏輯。

| 子模組 | 路徑 | 職責 |
|--------|------|------|
| `rpc/server` | `interfaces/rpc/server/` | tonic gRPC 伺服器（port 9001，可選 TLS） |
| `rpc/client` | `interfaces/rpc/client/` | gRPC 客戶端（自我測試 + 連接外部 Go 服務） |
| `web` | `interfaces/web/` | Axum HTTP 伺服器，REST API + backfill admin UI |
| `bot` | `interfaces/bot/` | Telegram bot 通知（告警、除息提醒） |

---

## 排程任務（UTC 時區）

| UTC 時間 | 台灣時間 | 任務 |
|---------|---------|------|
| 01:00 | 09:00 | 新興市場股票淨值更新 |
| 02:30 | 10:30 | 配息比例更新 |
| 03:00 | 11:00 | 季度 EPS 更新 |
| 04:00 | 12:00 | 季度財務報表更新 |
| 05:00 | 13:00 | 年度 EPS、財務報表、月營收、ISIN、下市股票 |
| 08:00 | 16:00 | 除息提醒、除權到帳提醒、公開申購提醒 |
| 09:00 | 17:00 | 持股權重比例更新、股價追蹤警示 |
| 15:00 | 23:00 | 收盤價抓取、估算股價計算 |
| 21:00 | 05:00+1 | 缺年度配息資料的股票補填 |
| 22:00 | 06:00+1 | 外資持股（QFII）更新 |

---

## 關鍵設計模式

### Keyable trait

Redis key 生成統一介面，避免 key 字串散落各處：

```rust
pub trait Keyable {
    fn key(&self) -> String;
    fn key_with_prefix(&self) -> String;
}
```

### Repository pattern

每個領域的 `repository.rs` 定義 `async_trait` Repository interface，
`infra/database/repository/` 提供 PostgreSQL 實作，
`domain` 層不直接依賴任何資料庫框架。

### 錯誤型別分層

| 層級 | 錯誤型別 | 說明 |
|------|---------|------|
| `infra::nosql` | `RedisError`（thiserror） | Redis 操作失敗 |
| `infra::database` | `RepositoryError`（thiserror） | DB 操作失敗，含 `NotFound`、`Conflict` |
| `infra::crawler` | `CrawlerError`（thiserror） | HTTP / 解析失敗，含 `NetworkError`、`ParseError` |
| `app` / `interfaces` | `anyhow::Result` | 跨子系統的 Use Case 邊界 |

### 日誌系統（tracing）

```
tracing::info!("...")
  ├─ FileLogLayer → LOGGER → log/YYYY-MM-DD_default_{level}.log（always-on）
  └─ fmt layer    → stdout（由 RUST_LOG 控制，預設靜音）
```

可選接入 Seq 結構化日誌平台（`app.json: logging.seq.server_url`）。

---

## 設定檔

| 檔案 | 用途 |
|------|------|
| `.env` | 本機開發用環境變數覆蓋（不提交版控） |
| `app.json` | 主設定：PostgreSQL、Redis、gRPC port、Telegram token、Seq URL |
| `deny.toml` | cargo-deny 供應鏈安全規則（授權清單、禁止重複依賴） |
| `etc/proto/` | Protocol Buffers 定義（basic / stock / control service） |

---

## CI 自動化

| Job | 工具 | 觸發條件 |
|-----|------|---------|
| fmt | `cargo fmt --check` | push / PR |
| check | `cargo check` | push / PR |
| clippy | `cargo clippy` | push / PR |
| build | `cargo build --release` | push / PR |
| test | `cargo test` | push / PR |
| deny | `cargo-deny` | push / PR |
| coverage | `cargo-llvm-cov` → LCOV artifact | push / PR |
| proto-lint | `buf-action` | push / PR |
