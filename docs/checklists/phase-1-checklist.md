# 階段一 (Phase 1): 核心層 (Core Layer) 抽取 Checklist

## 目標
將所有全域依賴、配置、宣告、日誌等抽取至獨立的 `core` 模組，並確保專案依然可以編譯。

## 執行項目
- [x] **建立 `src/core/` 目錄**
- [x] **搬移 `config` 模組**
  - [x] 將 `src/config.rs` 搬移至 `src/core/config.rs`
  - [x] 更新所有 `use crate::config` 引用
- [x] **搬移 `declare` 模組**
  - [x] 將 `src/declare.rs` 搬移至 `src/core/declare.rs`
  - [x] 更新所有 `use crate::declare` 引用
- [x] **搬移 `logging` 模組**
  - [x] 將 `src/logging/` 目錄搬移至 `src/core/logging/`
  - [x] 更新所有 `use crate::logging` 引用
- [x] **搬移 `util` 模組**
  - [x] 將 `src/util/` 目錄搬移至 `src/core/util/`
  - [x] 更新所有 `use crate::util` 引用
- [x] **配置 `src/core/mod.rs`**
  - [x] 將以上四個子模組在 `mod.rs` 內重新匯出 (`pub mod config;` 等)
- [x] **修改 `src/main.rs`**
  - [x] 移除舊有的 `pub mod config;`, `pub mod declare;`, `pub mod logging;`, `pub mod util;`
  - [x] 新增 `pub mod core;`
- [x] **編譯與驗證**
  - [x] 執行 `cargo check` (完全通過，解決了所有的 unresolved imports)
  - [x] 執行 `cargo build` (編譯成功)

## 備註
* 由於 `crate::` 引用了大量的舊路徑，透過自動化工具解析 `cargo check` 的錯誤，精準替換了超過 100 處路徑。
* `core` 層建立完成，目前的架構成功收攏了基礎的系統配置與工具庫。
