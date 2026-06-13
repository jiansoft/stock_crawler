# Phase 15 Checklist - 價格監控與追蹤領域 (Trace Domain) DDD 化

## A. 基本資訊
- **Phase**: Phase 15 - 價格監控與追蹤領域 DDD 化
- **Owner**: Antigravity
- **Branch**: `refactor/ddd-phase-15`
- **Start SHA**: `e7e54a0`
- **Rollback Tag**: `pre-phase-15`
- **預估完成日**: 2026-06-10

- [x] 分支已建立並切換至 `refactor/ddd-phase-15`
- [x] 歷史修改已確認無衝突

---

## B. 變更與重構清單

### 1. 定義 Domain 層新領域
- [x] 建立 `src/domain/trace/` 目錄：
  - [x] 建立 `src/domain/trace/mod.rs` 宣告與匯出子模組。
  - [x] 建立 `src/domain/trace/entity.rs` 定義 `PriceTrace` 領域實體與商業邏輯。
  - [x] 建立 `src/domain/trace/repository.rs` 定義 `TraceRepository` 倉儲合約。
- [x] 更新 `src/domain/mod.rs` 匯出 `pub mod trace;`。

### 2. 實作 Infrastructure 層之 Repository
- [x] 建立 `src/infra/database/repository/trace.rs`：
  - [x] 實作 `PgTraceRepository`，封裝 `"trace"` 表之讀寫與轉換邏輯。
- [x] 更新 `src/infra/database/repository/mod.rs` 匯出新的 Repository。

### 3. 重構 Application 層之 Use Cases
- [x] 重構 `src/app/event/trace/stock_price.rs`：
  - [x] 使用 `TraceRepository` 與 `PriceTrace` 替代原本直接呼叫 `Trace::fetch()`。
- [x] 重構並確認 `src/app/event/trace/price_tasks.rs` 中的測試或相依處是否有受影響。

---

## C. Gate 執行與驗證
- [x] `cargo fmt --all -- --check` 格式檢查通過
- [x] `cargo check --tests` 編譯通過
- [x] `cargo clippy --all-targets -- -D warnings` 零警告通過
- [x] `cargo test` 測試套件執行無誤
- [x] 新增單元測試驗證 `PgTraceRepository` 的行爲。

---

## D. 合併前確認
- [x] 與此重構無關的異動已排除
- [x] 重構後的繁體中文註解與 Rustdoc 已完整撰寫
- [x] PR 說明已附 Checklist 摘要與 Gate 結果
