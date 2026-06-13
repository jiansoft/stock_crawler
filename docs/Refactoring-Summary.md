# 重構摘要

更新日期：2026-06-13

## Phase 1 摘要

本次完成專案盤點與問題分析，未修改 Rust 程式碼、SQL、proto、設定檔或 CI。

## 新舊架構對照

| 項目 | 目前狀態 | 建議方向 |
|---|---|---|
| Crate 結構 | 單一 crate `stock_crawler` | 短期維持；中長期視邊界穩定度評估 workspace |
| 分層 | `core/domain/app/infra/interfaces` 已存在 | 收斂依賴方向，減少 app 直接依賴具體 infra |
| 啟動 | `main.rs` 組裝全部流程 | 拆 bootstrap/service lifecycle，但須先鎖定行為 |
| DB 存取 | table + repository 混合 | 新流程優先走 domain repository trait |
| Crawler | fetch/parser 混合，部分已 fixture 化 | 持續抽純 parser 與 fixture tests |
| Logging | 自製 logging + Seq + println/eprintln | 先統一呼叫點，再評估 tracing adapter |

## 修改檔案清單

僅新增文件：

- `docs/README.md`
- `docs/Architecture-Phase-1.md`
- `docs/Module-Analysis.md`
- `docs/Refactoring-Plan.md`
- `docs/Execution-Plan.md`
- `docs/Refactoring-Progress.md`
- `docs/Refactoring-Summary.md`
- `docs/Testing-Strategy.md`
- `docs/Performance-Notes.md`
- `docs/Risks-and-TODO.md`
- `docs/Documentation-Index.md`
- `docs/api/API-Guide.md`
- `docs/decisions/ADR-001-Phase-1-Keep-Single-Crate.md`
- `docs/diagrams/Architecture.mmd`
- `docs/diagrams/Dependency.mmd`

## 驗證結果

- Build：未執行。Phase 1 只做盤點與文件輸出。
- Test：未執行。Phase 2 才建立 baseline。
- Clippy：未執行。Phase 2 才建立 baseline。
- Benchmark：未發現 benches，未執行。

## 已知限制

- 本文件根據靜態閱讀與搜尋結果建立，未執行完整測試。
- 部分舊文件宣稱已完成的階段尚未逐項驗證。
- 外部 API 與 DB/Redis 相關測試需在 Phase 2 實際跑過後更新結論。
