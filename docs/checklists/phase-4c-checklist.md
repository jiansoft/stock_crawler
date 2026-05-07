# Phase 4c: crawler -> infra Checklist

## A. 基本資訊
- [x] Phase 編號與名稱：Phase 4c: crawler -> infra
- [x] 負責人：Codex
- [x] 分支名稱：refactor/stage-4a-infra-cache-nosql
- [x] 起始 commit SHA：2ae36b39f95285d9f305f3bb7ad9ecfcad2be0b1
- [ ] 回滾 tag 已建立：pre-stage-4c

## B. 搬移清單 (Move List)
- [x] 搬移 `src/crawler/` -> `src/infra/crawler/`
- [x] `src/infra/mod.rs` 新增 `pub mod crawler;`
- [x] `src/main.rs` 移除 `pub mod crawler;`

## C. 路徑修正清單 (use/module path)
- [x] 全域替換規則：`crate::crawler::` -> `crate::infra::crawler::`
- [x] `infra/crawler` 內部自引用路徑已完成修正（含 `price_tasks.rs`、`share.rs` 與站點模組）
- [x] app / interfaces / infra 受影響引用已完成編譯修正
- [x] warning 清理：移除 `src/app/event/taiwan_stock/public.rs` 未使用 `infra::crawler` import

## D. Gate 執行結果
- [x] `cargo check` 通過（2026-05-07）
- [x] `cargo build` 通過（2026-05-07）
- [ ] (輔助) 完整啟動 + gRPC 自測 + Redis ping（有環境再執行）

## E. 中斷交接資訊 (Resume)
- Last Update Time: 2026-05-07 15:02:42
- Stopped At: Phase 4c Gate Completed
- Next Action: 進入 Phase 5（`infra/database/table` 領域化分組）。
