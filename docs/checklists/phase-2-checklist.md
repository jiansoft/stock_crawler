# Phase 2: 抽出 interfaces (外部入口集中) Checklist

## A. 基本資訊
- [x] Phase 編號與名稱：Phase 2: 抽出 interfaces
- [x] 負責人：Gemini CLI
- [x] 分支名稱：refactor/stage-2-interfaces
- [x] 起始 commit SHA：5d1f4666c305685bfddf74375eba8737925d708a
- [x] 回滾 tag 已建立：pre-stage-2

## B. 搬移清單 (Move List)
- [x] 建立 `src/interfaces/` 目錄
- [x] 搬移 `src/rpc/` -> `src/interfaces/rpc/`
- [x] 搬移 `src/web/` -> `src/interfaces/web/`
- [x] 搬移 `src/bot/` -> `src/interfaces/bot/`
- [x] 建立 `src/interfaces/mod.rs` 並宣告子模組
- [x] `src/main.rs` 移除舊 `mod` 宣告並新增 `pub mod interfaces;`
- [x] 若有 `#[cfg(test)]` 條件編譯模組，目標 `mod.rs` 已加上

## C. 路徑修正清單 (use/module path)
- [x] 全域替換規則：`crate::rpc` -> `crate::interfaces::rpc`
- [x] 全域替換規則：`crate::web` -> `crate::interfaces::web`
- [x] 全域替換規則：`crate::bot` -> `crate::interfaces::bot`
- [x] `build.rs` 中的 `OUT_DIR` 從 `"src/rpc"` 修改為 `"src/interfaces/rpc"`
- [x] `src/interfaces/rpc/mod.rs` 中的 `include!` 產碼檔路徑確認
- [x] 檢查並修正多重 import (`use crate::{...}`)

## D. Gate 執行結果
- [x] `cargo check` 通過
- [x] `cargo build` 通過
- [ ] (輔助) 服務啟動驗證

## E. 中斷交接資訊 (Resume)
- Last Update Time: 2026-05-07
- Stopped At: Phase 2 Completed
- Next Action: Merge branch and start Phase 3.
