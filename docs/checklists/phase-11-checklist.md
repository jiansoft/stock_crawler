# Phase 11: 個股報價與歷史價格領域 (Quote & Price Domain) DDD 化 Checklist

## A. 基本資訊
- [x] Phase 編號與名稱：Phase 11: 個股報價與歷史價格領域 DDD 化
- [x] 負責人：Antigravity
- [x] 分支名稱：`refactor/ddd-phase-11`
- [x] 起始 commit SHA：28fb440abed629c42cbf130f146cde6d2679c78d
- [x] 回滾 tag 已建立：pre-stage-11

## B. 設計與定義 (Domain & Repository Definition)
- [x] **建立 `src/domain/quote/` 目錄**
  - [x] 建立 `src/domain/quote/mod.rs` 宣告子模組
  - [x] 建立 `src/domain/quote/entity.rs` 定義領域實體與商業邏輯
  - [x] 建立 `src/domain/quote/repository.rs` 定義 `QuoteRepository` 倉儲介面
- [x] **更新 `src/domain/mod.rs`**
  - [x] 匯出 `pub mod quote;`

## C. 基礎設施實現 (Infrastructure Implementation)
- [x] **實作 `PgQuoteRepository`**
  - [x] 於 `src/infra/database/repository/quote.rs` 實現 `QuoteRepository` 介面
  - [x] 封裝 `daily_quote/mod.rs`、`last_daily_quotes.rs` 與 `daily_stock_price_stats.rs` 的 SQL 行為
  - [x] **快取封裝**：將原有 `SHARE` 快取與 Redis `nosql::redis` 呼叫整合至 Repository 內部，對外屏蔽快取策略
- [x] **更新 `src/infra/database/repository/mod.rs`**
  - [x] 匯出新的 Repository 模組

## D. 應用層與事件重構 (Application Layer & Events Refactoring)
- [x] **重構 `src/app/event/trace/stock_price.rs` (個股價格追蹤監控)**
  - [x] 改為注入與呼叫 `QuoteRepository` 取代直接讀取全域快取或 DB Table 的行為
- [x] **重構日報價與歷史報價之爬蟲與回補流程**
  - [x] 改造 `app/backfill/daily_quote/` 系列任務，透過 `QuoteRepository` 進行批次儲存，不再直連 Table UPSERT
- [x] **重構報價統計與均線相關計算**
  - [x] 改造均線或 PBR 運算，以倉儲獲取資料

## E. 單元測試與驗證 (Testing & Validation)
- [x] **撰寫領域邏輯與 Repository 單元測試**
  - [x] 為 `DailyQuote` 漲跌幅計算與波動度判定撰寫單元測試
  - [x] 為 `PgQuoteRepository` 的 Cache-Aside 合約行為（快取命中/未命中回寫）撰寫單元與整合測試
- [x] **Gate 執行結果**
  - [x] `cargo check` 通過
  - [x] `cargo build` 通過
  - [x] `cargo test` 通過 (確認無 regression)
