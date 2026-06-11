# Phase 16 Checklist - 系統設定領域 (Config Domain) DDD 化

## A. 基本資訊
- **Phase**: Phase 16 - 系統設定領域 DDD 化
- **Owner**: Antigravity
- **Branch**: `refactor/ddd-phase-16`
- **Start SHA**: `6cbf8f5`
- **Rollback Tag**: `pre-phase-16`
- **預估完成日**: 2026-06-10

- [x] 分支已建立並切換至 `refactor/ddd-phase-16`
- [x] 歷史修改已確認無衝突

---

## B. 變更與重構清單

### 1. 定義 Domain 層新領域
- [x] 建立 `src/domain/config/` 目錄：
  - [x] 建立 `src/domain/config/mod.rs` 宣告與匯出子模組。
  - [x] 建立 `src/domain/config/entity.rs` 定義 `SystemConfig` 領域實體與商業邏輯。
  - [x] 建立 `src/domain/config/repository.rs` 定義 `ConfigRepository` 倉儲合約。
- [x] 更新 `src/domain/mod.rs` 匯出 `pub mod config;`。

### 2. 實作 Infrastructure 層之 Repository
- [x] 建立 `src/infra/database/repository/config.rs`：
  - [x] 實作 `PgConfigRepository`，封裝 `"config"` 表之讀寫與轉換邏輯。
- [x] 更新 `src/infra/database/repository/mod.rs` 匯出新的 Repository。

### 3. 重構 Application 層之 Use Cases
- [x] 重構所有調用舊 `Config` table 結構的地方，改為呼叫 `ConfigRepository`：
  - [x] 檢查並更新 `src/app/backfill/` 下相關流程。
  - [x] 檢查並更新 `src/app/event/` 下相關流程。

---

## C. Gate 執行與驗證
- [x] `cargo fmt --all -- --check` 格式檢查通過
- [x] `cargo check --tests` 編譯通過
- [x] `cargo clippy --all-targets -- -D warnings` 零警告通過
- [x] `cargo test` 測試套件執行無誤
- [x] 新增單元測試驗證 `PgConfigRepository` 的行爲。

---

## D. 合併前確認
- [x] 與此重構無關的異動已排除
- [x] 重構後的繁體中文註解與 Rustdoc 已完整撰寫
- [x] PR 說明已附 Checklist 摘要與 Gate 結果
