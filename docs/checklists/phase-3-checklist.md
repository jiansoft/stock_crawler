# Phase 3: 抽出 app (流程編排集中) Checklist

## A. 基本資訊
- [x] Phase 編號與名稱：Phase 3: 抽出 app
- [x] 負責人：Gemini CLI
- [x] 分支名稱：refactor/stage-3-app
- [x] 起始 commit SHA：50b548300ffff002bfb7d36da24171c3a8b1c000
- [x] 回滾 tag 已建立：pre-stage-3

## B. 搬移清單 (Move List)
- [x] 建立 `src/app/` 目錄
- [x] 搬移 `src/scheduler.rs` -> `src/app/scheduler.rs`
- [x] 搬移 `src/manual_backfill.rs` -> `src/app/manual_backfill.rs`
- [x] 搬移 `src/backfill/` -> `src/app/backfill/`
- [x] 搬移 `src/event/` -> `src/app/event/`
- [x] 搬移 `src/calculation/` -> `src/app/calculation/`
- [x] 建立 `src/app/mod.rs` 並宣告子模組
- [x] `src/main.rs` 移除舊 `mod` 宣告並新增 `pub mod app;`
- [x] `src/app/mod.rs` 中針對 `manual_backfill` 使用 `#[cfg(test)] mod manual_backfill;`

## C. 路徑修正清單 (use/module path)
- [x] 全域替換規則：`crate::scheduler` -> `crate::app::scheduler`
- [x] 全域替換規則：`crate::backfill` -> `crate::app::backfill`
- [x] 全域替換規則：`crate::event` -> `crate::app::event`
- [x] 全域替換規則：`crate::calculation` -> `crate::app::calculation`
- [x] `src/main.rs` 啟動點改為 `crate::app::scheduler::start(...)`
- [x] 修正 `event/taiwan_stock/closing.rs` 內的 `crate::event::trace` 引用
- [x] 修正 `manual_backfill.rs` 內的測試註解說明

## D. Gate 執行結果
- [x] `cargo check` 通過
- [x] `cargo build` 通過
- [ ] (輔助) 手動觸發最小 backfill 流程驗證

## E. 中斷交接資訊 (Resume)
- Last Update Time: 2026-05-07
- Stopped At: Phase 3 Merged to main
- Next Action: Start Phase 4a (cache + nosql -> infra).
