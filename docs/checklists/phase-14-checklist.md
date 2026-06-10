# Phase 14 Checklist - 指數與市場指數領域 (Index Domain) DDD 化

## A. 基本資訊
- **Phase**: Phase 14 - 指數與市場指數領域 DDD 化
- **Owner**: Antigravity
- **Branch**: `refactor/ddd-phase-14`
- **Start SHA**: `e29a362`
- **Rollback Tag**: `pre-phase-14`
- **預估完成日**: 2026-06-10

- [x] 分支已建立並切換至 `refactor/ddd-phase-14`
- [x] 歷史修改已確認無衝突

---

## B. 變更與重構清單

### 1. 定義 Domain 層新領域
- [x] 建立 `src/domain/market_index/` 目錄：
  - [x] 建立 `src/domain/market_index/mod.rs` 宣告與匯出子模組。
  - [x] 建立 `src/domain/market_index/entity.rs` 定義 `MarketIndex` 領域實體與商業邏輯。
  - [x] 建立 `src/domain/market_index/repository.rs` 定義 `MarketIndexRepository` 倉儲合約。
- [x] 更新 `src/domain/mod.rs` 匯出 `pub mod market_index;`。

### 2. 實作 Infrastructure 層之 Repository
- [x] 建立 `src/infra/database/repository/market_index.rs`：
  - [x] 實作 `PgMarketIndexRepository`，封裝 `"index"` 表之讀寫與轉換邏輯。
- [x] 更新 `src/infra/database/repository/mod.rs` 匯出新的 Repository。
- [x] 更新快取同步層 `SHARE`：
  - [x] 修改 `src/infra/cache/share.rs`，使快取結構中的 `Index` 使用全新的 `MarketIndex` 領域模型或對齊。

### 3. 重構 Application 層之 Use Cases
- [x] 重構 `src/app/backfill/taiwan_stock_index.rs`：
  - [x] 使用 `MarketIndexRepository` 與 `MarketIndex` 替代原本直接呼叫 `Index::upsert` 與 table 結構。
- [x] 重構 `src/app/backfill/acl.rs` 中的 `IndexAclMapper`：
  - [x] 調整其 `from_command` 方法使其輸出為領域實體 `MarketIndex`。

### 4. 重構手動回補與其它依賴處
- [x] 重構 `src/app/manual_backfill.rs` 中的 `test_backfill_taiwan_stock_index` 以適配全新倉儲。

---

## C. Gate 執行與驗證
- [x] `cargo fmt --all -- --check` 格式檢查通過
- [x] `cargo check --tests` 編譯通過
- [x] `cargo clippy --all-targets -- -D warnings` 零警告通過
- [x] `cargo test` 測試套件執行無誤
- [x] 新增單元測試驗證 `PgMarketIndexRepository` 的行爲。

---

## D. 合併前確認
- [x] 與此重構無關的異動已排除
- [x] 重構後的繁體中文註解與 Rustdoc 已完整撰寫
- [x] PR 說明已附 Checklist 摘要與 Gate 結果
