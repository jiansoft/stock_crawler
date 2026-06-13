# stock_crawler 文件入口

更新日期：2026-06-13

本目錄保存 `stock_crawler` 的架構、重構、測試與風險紀錄。本文是文件入口；詳細索引請見 [Documentation-Index.md](Documentation-Index.md)。

## 目前狀態

- 本專案目前是單一 Rust crate，尚不是 Cargo workspace。
- `src/` 已依 `core`、`domain`、`app`、`infra`、`interfaces` 分層。
- 本次盤點屬於 Phase 1：專案盤點與問題分析；未修改 Rust 程式碼、SQL、proto、設定檔或 CI。
- 舊文件 `docs/architecture.md`、`docs/refactor_ddd_continuation_plan.md` 與 `docs/checklists/` 仍保留；其內容是否完全符合目前程式碼：待確認（To Be Verified）。

## 建議閱讀順序

1. [Architecture-Phase-1.md](Architecture-Phase-1.md)
2. [Module-Analysis.md](Module-Analysis.md)
3. [Refactoring-Plan.md](Refactoring-Plan.md)
4. [Execution-Plan.md](Execution-Plan.md)
5. [Testing-Strategy.md](Testing-Strategy.md)
6. [Risks-and-TODO.md](Risks-and-TODO.md)
7. [Refactoring-Progress.md](Refactoring-Progress.md)
