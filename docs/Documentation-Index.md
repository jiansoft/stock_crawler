# Documentation Index

更新日期：2026-06-13

| File | Purpose | 狀態 |
|-------|----------|------|
| `docs/README.md` | 文件入口與閱讀順序。 | ✅ Active |
| `docs/Architecture-Phase-1.md` | 目前專案結構、架構圖、CI/CD、Docker 與依賴方向盤點。Windows 檔案系統大小寫不敏感，為避免覆蓋既有 `docs/architecture.md`，使用此檔名。 | ✅ Active |
| `docs/Module-Analysis.md` | 模組職責、相依關係、技術棧、問題與高風險區域分析。 | ✅ Active |
| `docs/Refactoring-Plan.md` | 行為等價重構方向、分層建議、順序與回滾策略。 | ✅ Active |
| `docs/Execution-Plan.md` | AI 執行重構任務的入口 SOP，包含分支、baseline、PR、驗證與回滾流程。 | ✅ Active |
| `docs/Refactoring-Progress.md` | 每階段進度、修改檔案、驗證結果與待辦。 | ✅ Active（需持續更新） |
| `docs/Refactoring-Summary.md` | 重構摘要、新舊架構對照、驗證與限制。 | ✅ Active |
| `docs/Testing-Strategy.md` | Baseline commands、測試分類與 characterization tests 策略。 | ✅ Active |
| `docs/Performance-Notes.md` | 效能敏感路徑、allocator 設定、clone/allocation 觀察與 benchmark 建議。 | ✅ Active |
| `docs/Risks-and-TODO.md` | 高風險區、待確認事項與 Phase 2 待辦。 | ✅ Active |
| `docs/api/API-Guide.md` | 對外介面概要（gRPC 方法列表、HTTP 端點、Telegram 觸發時機）與重構限制。 | ✅ Active |
| `docs/decisions/ADR-001-Phase-1-Keep-Single-Crate.md` | Phase 1 暫不拆 workspace 的架構決策。 | ✅ Active |
| `docs/diagrams/Architecture.mmd` | Mermaid 架構圖。 | ✅ Active（部分語義待釐清，見圖內說明） |
| `docs/diagrams/Dependency.mmd` | Mermaid 依賴方向圖與風險註記。 | ✅ Active |
| `docs/architecture.md` | 既有架構指南；分層設計原則仍有效，部分目錄結構說明已由 `Architecture-Phase-1.md` 更新。 | ⚠️ Partially Superseded by `Architecture-Phase-1.md` |
| `docs/refactor_ddd_continuation_plan.md` | Phase 18-20 計畫；所有 Phase 已標記 ✅ 完成。 | ✅ Completed |
| `docs/refactor_staged_plan_zh_tw.md` | Phase 0~6 分階段重構計畫；Phase 6 後由 `refactor_ddd_continuation_plan.md` 延續。 | ✅ Completed（Phase 0~6） |
| `docs/manual-backfill-web.md` | 手動回補 Web UI / HTTP API / gRPC 操作手冊。 | ✅ Active（路徑已更新至現行路徑） |
| `docs/github_actions_recommendations.md` | GitHub Actions 改善建議清單。 | 📋 Draft（尚未實作） |
