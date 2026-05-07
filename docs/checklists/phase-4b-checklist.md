# Phase 4b: database -> infra Checklist

## A. 基本資訊
- [x] Phase 編號與名稱：Phase 4b: database -> infra
- [x] 負責人：Codex
- [x] 分支名稱：refactor/stage-4a-infra-cache-nosql
- [x] 起始 commit SHA：5c1a3e6d0c80b607f572dc6d76ec6f8c0cf20c00
- [ ] 回滾 tag 已建立：pre-stage-4b

## B. 搬移清單 (Move List)
- [x] 搬移 `src/database/` -> `src/infra/database/`
- [x] `src/infra/mod.rs` 新增 `pub mod database;`
- [x] `src/main.rs` 移除 `pub mod database;`

## C. 路徑修正清單 (use/module path)
- [x] 全域替換規則：`crate::database::` -> `crate::infra::database::`
- [x] 主要受影響模組已完成編譯修正（app / interfaces / crawler / infra/database）
- [x] `infra/database` 內部 `use` 與 `database::get_connection/get_tx` 自引用已修正

## D. Gate 執行結果
- [x] `cargo check` 通過（2026-05-07）
- [x] `cargo build` 通過（2026-05-07）
- [ ] (輔助) 啟動 smoke test（有環境再執行）

## E. 中斷交接資訊 (Resume)
- Last Update Time: 2026-05-07 14:45:37
- Stopped At: Phase 4b Gate Completed
- Next Action: 進入 Phase 4c（crawler -> infra）。
