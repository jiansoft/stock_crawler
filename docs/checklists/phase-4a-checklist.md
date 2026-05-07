# Phase 4a: cache + nosql -> infra Checklist

## A. 基本資訊
- [x] Phase 編號與名稱：Phase 4a: cache + nosql -> infra
- [x] 負責人：Codex
- [x] 分支名稱：refactor/stage-4a-infra-cache-nosql
- [x] 起始 commit SHA：5c1a3e6d0c80b607f572dc6d76ec6f8c0cf20c00
- [x] 回滾 tag 已建立：pre-stage-4a

## B. 搬移清單 (Move List)
- [x] 建立 `src/infra/`
- [x] 搬移 `src/cache/` -> `src/infra/cache/`
- [x] 搬移 `src/nosql/` -> `src/infra/nosql/`
- [x] 建立 `src/infra/mod.rs`
- [x] `src/main.rs` 移除舊 `pub mod cache;` / `pub mod nosql;`，新增 `pub mod infra;`

## C. 路徑修正清單 (use/module path)
- [x] 全域替換規則：`crate::cache::` -> `crate::infra::cache::`
- [x] 全域替換規則：`crate::nosql::` -> `crate::infra::nosql::`
- [x] `main.rs` 啟動流程改用 `infra::cache::SHARE` 與 `infra::nosql::redis::CLIENT`
- [x] 主要受影響模組已完成編譯修正（app / interfaces / crawler / database）
- [ ] 清理本階段引入的 `unused import` 警告（不阻擋 Gate）

## D. Gate 執行結果
- [x] `cargo check` 通過（2026-05-07）
- [x] `cargo build` 通過（2026-05-07）
- [ ] (輔助) 啟動 smoke test（有環境再執行）

## E. 中斷交接資訊 (Resume)
- Last Update Time: 2026-05-07 14:32:15
- Stopped At: Phase 4a Gate Completed
- Next Action: 清理 Phase 4a 警告後送 PR；合併後進入 Phase 4b（database -> infra）。
