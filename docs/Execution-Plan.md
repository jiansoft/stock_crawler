# AI 重構執行計畫

更新日期：2026-06-13

本文件是讓 AI 執行重構任務時的入口。AI 開始任何 Phase 前，必須先閱讀本文件與下列文件：

- [Refactoring-Plan.md](Refactoring-Plan.md)
- [Testing-Strategy.md](Testing-Strategy.md)
- [Risks-and-TODO.md](Risks-and-TODO.md)
- [Module-Analysis.md](Module-Analysis.md)
- [Refactoring-Progress.md](Refactoring-Progress.md)

## 執行總原則

- 本專案重構是 Behavior-Preserving Refactoring，所有既有功能與對外行為必須保持一致。
- 不得直接在 `main` 上執行重構。
- 每個 Phase、模組或主題都建立短分支，例如 `refactor/phase-2-baseline`。
- 每個 PR 只處理一個 Phase、模組或主題。
- 不得一次性重寫整個專案。
- 不得修改 public API、CLI 使用方式、輸出格式、serialization 格式、資料庫 schema、migration、權限邏輯或設定檔預設行為。
- 若無法以測試、執行結果或程式碼分析證明行為一致，必須停止修改並回報風險。
- 用途不明的區域標示 `待確認（To Be Verified）`，不得猜測修改。

## AI 每次執行任務的 SOP

1. 讀取 [Refactoring-Progress.md](Refactoring-Progress.md)，確認目前 Phase 與已完成事項。
2. 讀取 [Risks-and-TODO.md](Risks-and-TODO.md)，避開高風險與待確認區域。
3. 讀取 [Testing-Strategy.md](Testing-Strategy.md)，確認本次修改前後要執行的驗證。
4. 讀取 [Module-Analysis.md](Module-Analysis.md)，確認目標模組職責與相依關係。
5. 依 [Refactoring-Plan.md](Refactoring-Plan.md) 選擇下一個 Phase、模組或主題。
6. 建立短分支，例如：

```bash
git checkout -b refactor/phase-2-baseline
```

7. 修改前執行 baseline 驗證，至少包含：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

8. 若 baseline 失敗，先記錄失敗原因與環境條件，不得直接進入核心重構。
9. 若測試不足，先補 characterization tests。
10. 每次只修改一個模組或一個主題。
11. 修改完成後執行：

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

12. 若有 benchmark，執行：

```bash
cargo bench
```

13. 更新 [Refactoring-Progress.md](Refactoring-Progress.md)，記錄日期、模組、修改檔案、原因、驗證結果與待辦。
14. 若有重大架構決策，新增 ADR 到 `docs/decisions/`。
15. 建立 PR，PR 內容必須包含目的、影響範圍、修改檔案、驗證命令與結果、已知風險。
16. PR 合併回 `main` 後，才能開始下一個 Phase 或主題。

## PR 必填內容

```markdown
## Purpose

## Affected Modules

## Changed Files

## Behavior Compatibility

## Verification

## Risks

## Rollback Plan
```

## 回滾規則

- 若修改後測試失敗，先判斷是環境問題、既有 baseline 問題，或本次修改造成。
- 若是本次修改造成，優先回滾本次分支中的相關 commit。
- 不得使用破壞性命令回滾使用者未確認的變更。
- SQL、proto、設定檔預設值不得與一般重構混在同一 PR。

## 下一步

目前下一個 Phase 是 Phase 2：建立行為保護。

Phase 2 開始前，AI 必須先執行 baseline 驗證，並將結果更新到 [Refactoring-Progress.md](Refactoring-Progress.md)。

