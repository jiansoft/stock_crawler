# Phase 17 Checklist - 殖利率排行領域 (Yield Rank Domain) DDD 化

## A. 基本資訊
- **Phase**: Phase 17 - 殖利率排行領域 DDD 化
- **Owner**: Antigravity
- **Branch**: `refactor/ddd-phase-17`
- **Start SHA**: `e473e04`
- **Rollback Tag**: `pre-phase-17`
- **預估完成日**: 2026-06-11

- [x] 分支已建立並切換至 `refactor/ddd-phase-17`
- [x] 歷史修改已確認無衝突

---

## B. 變更與重構清單

### 1. 定義 Domain 層新領域
- [x] 建立 `src/domain/yield_rank/` 目錄：
  - [x] 建立 `src/domain/yield_rank/mod.rs` 宣告與匯出子模組。
  - [x] 建立 `src/domain/yield_rank/entity.rs` 定義 `YieldRank` 領域實體與商業邏輯。
  - [x] 建立 `src/domain/yield_rank/repository.rs` 定義 `YieldRankRepository` 倉儲合約。
- [x] 更新 `src/domain/mod.rs` 匯出 `pub mod yield_rank;`。

### 2. 實作 Infrastructure 層之 Repository
- [x] 建立 `src/infra/database/repository/yield_rank.rs`：
  - [x] 實作 `PgYieldRankRepository`，封裝 `"yield_rank"` 表之讀寫與轉換邏輯。
- [x] 更新 `src/infra/database/repository/mod.rs` 匯出新的 Repository。

### 3. 重構 Application 層之 Use Cases
- [x] 重構所有調用舊 `YieldRank` table 結構的地方，改為呼叫 `YieldRankRepository`：
  - [x] 檢查並更新 `src/app/event/taiwan_stock/closing.rs` 中重建與寫入的流程。

---

## C. Gate 執行與驗證
- [x] `cargo fmt --all -- --check` 格式檢查通過
- [x] `cargo check --tests` 編譯通過
- [x] `cargo clippy --all-targets -- -D warnings` 零警告通過
- [x] `cargo test` 測試套件執行無誤
- [x] 新增單元測試驗證 `PgYieldRankRepository` 的行爲。

---

## D. 合併前確認
- [x] 與此重構無關的異動已排除
- [x] 重構後的繁體中文註解與 Rustdoc 已完整撰寫
- [x] PR 說明已附 Checklist 摘要與 Gate 結果
