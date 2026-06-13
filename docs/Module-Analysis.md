# 模組分析

更新日期：2026-06-13

## 模組職責與相依關係

| 模組 | 職責 | 主要相依 | 觀察 |
|---|---|---|---|
| `src/main.rs` | 載入設定、初始化 logging、DB ping、cache load、scheduler、gRPC、Web、signal handling、Redis ping；Linux musl 下使用 **mimalloc** 全域 allocator 並在 `.init_array` 調整環境參數；啟動後呼叫 `test_client::run_test()` 自我驗證 gRPC 連線（失敗會觸發 Telegram 告警） | `core`、`infra`、`app`、`interfaces`、`mimalloc`（Linux musl only） | 組裝根過重，建議後續拆出 startup/bootstrap 用例。 |
| `core/config.rs` | app.json 與 env 設定載入 | `config`、`dotenv`、`serde` | 有 `expect` fail-fast 行為，重構時不得改變預設/錯誤行為。 |
| `core/declare.rs` | 共用 enum、常數、交易所/產業/季度等基礎型別 | `chrono`、`strum` | 被多層使用，屬於核心穩定區。 |
| `core/logging/` | 非同步檔案日誌、輪替、Seq 轉送 | `tokio`、`reqwest`、`serde_json` | 專案尚未全面使用 `tracing`，目前自製 logging 是高影響區。 |
| `core/util/` | 日期、文字、HTTP、診斷與通用工具 | `reqwest`、`chrono`、`scraper` | parser 與日期推論測試已有覆蓋，但仍是行為敏感區。 |
| `domain/*/entity.rs` | 股票、報價、財務、股利、資金流、投資組合等領域實體 | `core`、`chrono`、`rust_decimal` | 部分 default date 使用 `unwrap()` 建立常數日期，需小心不改語意。 |
| `domain/*/repository.rs` | repository trait 合約 | `anyhow::Result` | 目前 trait 使用 `anyhow`，錯誤型別尚未 domain-specific。 |
| `domain/events.rs` | 領域事件 | domain entity / primitive | 可作為後續事件派發解耦核心。 |
| `app/scheduler.rs` | cron job 註冊與啟動補償 | `tokio-cron-scheduler`、`app/event` | 排程時間屬對外行為，重構需 characterization tests。 |
| `app/backfill/` | 歷史資料回補、ACL mapper、外部 DTO 到 domain command/entity 轉換 | `infra/crawler`、`infra/database`、`infra/cache`、`domain` | `acl.rs` 與多個回補模組偏大，是優先候選但需測試保護。 |
| `app/calculation/` | 股利、估價、每日報價與資金歷史計算 | `domain`、`infra/database` | 計算規則屬業務核心，不建議早期大改。 |
| `app/event/` | 排程事件處理、盤後流程、價格追蹤 | `infra`、`interfaces/bot`、`domain/events` | `handlers.rs`、`trace/price_tasks.rs` 偏大且耦合通知、RPC、Redis。 |
| `infra/crawler/` | 外部資料來源 HTTP fetch 與 parser | `core/util/http`、`reqwest`、`scraper`、`serde_json` | 多數 parser 已有單元測試；部分測試仍連真實網路。 |
| `infra/cache/` | 共享記憶體快取、TTL、即時報價快照 | `RwLock`、`moka`、`domain`、`database` | `share.rs` 很大，初始化與查詢混合。 |
| `infra/nosql/redis.rs` | Redis pool 與 get/set/delete helper | `deadpool-redis`、`anyhow` | 測試需 Redis；錯誤訊息與序列化行為需鎖定。 |
| `infra/database/mod.rs` | PostgreSQL pool、連線與 ping | `sqlx`、`core/config` | 啟動依賴核心，連線行為不可改。 |
| `infra/database/table/` | table model、SQL CRUD、延伸查詢 | `sqlx`、`domain` | 多個 God Module，如 `quote/daily_quote/mod.rs`、`dividend/mod.rs`。 |
| `infra/database/repository/` | PostgreSQL repository 實作與 domain/table mapping | `domain/*/repository`、`table/*` | 建議持續作為 app 與 table 之間的邊界。 |
| `interfaces/rpc/` | gRPC stub、client、server service | `tonic`、`prost`、`app`、`domain` | generated files 不可手改；service 可逐步瘦身。 |
| `interfaces/web/` | Axum HTTP server 與手動回補 API/UI | `axum`、`app/backfill`、`tokio` | `backfill_admin.rs` 偏大，狀態與 handler 混合。 |
| `interfaces/bot/telegram.rs` | Telegram send、alert、fallback | `reqwest`、`core/config` | 應視為 interface adapter；infra 不宜直接依賴。 |

## 技術分析

| 類別 | 使用技術 |
|---|---|
| Runtime | Tokio `full`、tokio-cron-scheduler、tokio-retry |
| Database | PostgreSQL + SQLx 0.9、SQL files in `etc/sql/` |
| Cache / NoSQL | Redis via deadpool-redis、in-memory cache via Moka/RwLock |
| Web / RPC | Axum 0.8、Tonic 0.14、Prost 0.14 |
| Crawling / HTTP | Reqwest 0.13、Scraper、Regex、encoding_rs |
| Logging | 自製 `core::logging` 檔案日誌與 Seq forwarding，仍有 `println!` / `eprintln!` |
| Error Handling | 大量 `anyhow::Result`、少量 `Box<dyn Error>`、尚未使用 `thiserror` |
| Config | `config` crate + `dotenv` + `app.json` + env overrides |
| Serialization | Serde、serde_json、prost |
| Numeric / Date | rust_decimal、chrono、time |
| Parallelism | Tokio async、futures stream、Rayon、num_cpus |
| Memory (Linux musl) | **mimalloc** 全域 allocator（`#[global_allocator]`），搭配 `.init_array` 環境參數最佳化，詳見 `Performance-Notes.md` |

## 問題分析

- 高耦合：`main.rs`、`app/event/handlers.rs`、`app/backfill/acl.rs`、`interfaces/web/backfill_admin.rs` 同時處理流程、外部 adapter、錯誤與通知。
- God Module：`infra/database/table/quote/daily_quote/mod.rs`、`infra/cache/share.rs`、`infra/database/table/dividend/mod.rs`、`app/event/handlers.rs`、`app/backfill/acl.rs`、`infra/crawler/mod.rs`。
- Clone 過多：DTO/domain mapper 與 async task spawn 需要 clone，但部分 `String`/Vec clone 可能只是借用設計不足。需以 profiler 或 targeted review 驗證，不應盲改。
- Ownership 問題：共享全域 `SHARE`、Redis `CLIENT`、多處 `Arc`/`RwLock` 讓測試隔離與生命週期管理變困難。
- Error Handling 不一致：`anyhow`、`Box<dyn Error>`、`unwrap/expect`、字串化錯誤並存；domain trait 直接使用 `anyhow`。
- Logging 不一致：正式路徑使用 `core::logging`，但仍有 `println!`、`eprintln!`；轉 `tracing` 前需保留檔案與 Seq 行為。
- 測試不足或不穩：有 colocated tests，但 DB/Redis/live-network 混合；部分外部 API parser 測試仍需 fixture 化與 skip 策略。
- 文件不足：已有歷史文件，但缺少統一索引、ADR、Phase 1 當前狀態快照與風險清單。
- 效能風險：啟動一次載入共享快取、盤中即時報價背景任務、巨大 HashMap/RwLock、Redis invalidation 與多來源 HTTP fallback。

## 高風險區域

不建議立即修改：

- `core/config.rs`：設定預設與 env 覆蓋是啟動契約。
- `core/logging/`：目前承載檔案與 Seq 行為。
- `src/interfaces/rpc/*.rs` generated files：只應改 proto 後重建。
- `etc/sql/`：schema / migration 不屬本次行為等價重構範圍。
- `app/event/taiwan_stock/*` 排程入口：排程時間與通知是使用者可觀察行為。
- `infra/crawler/*` live API 呼叫：外部網站行為不穩，需 fixture 保護後再拆。

需額外驗證：

- `Dockerfile` 是否仍被部署流程或外部 pipeline 使用：待確認（To Be Verified）。
- `failed_log.txt` / `failed_log2.txt` 是否為暫存除錯輸出，是否應納入 `.gitignore`：待確認（To Be Verified）。
- README 中提到已移除 DNS 設定，但目前實際 `app.json` 已無 afraid/dynu/noip；舊文件是否過時：待確認（To Be Verified）。

