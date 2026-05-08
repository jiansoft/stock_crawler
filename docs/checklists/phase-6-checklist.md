#### A. 基本資訊

- [x] Phase 編號與名稱已填寫
- [x] 負責人已填寫
- [x] 分支名稱已填寫（`refactor/stage-6-finalize`）
- [x] 起始 commit SHA 已填寫
- [x] 回滾 tag 已建立（例如 `pre-stage-6`）

建議填寫欄位：

- `Phase`：Phase 6 收尾與規範固化
- `Owner`：AI Agent
- `Branch`：refactor/stage-6-finalize
- `Start SHA`：64ed675
- `Rollback Tag`：pre-stage-6
- `預估完成日`：2026-05-08

#### B. 搬移清單（Move List）

本階段為收尾與清理，無新增模組搬移。
- [x] 刪除 `Rocket.toml`
- [x] 清理 `main.rs` 中的 Rocket 殘留註解與公式註解

#### C. 路徑修正清單（use/module path）

本階段無路徑修正，主要為文件與 CI 更新：
- [x] `README.md` 更新架構圖與導覽
- [x] 新增 `docs/architecture.md` (新模組放置規範)
- [x] `rust.yml` 更新 CI 檢查步驟 (`fmt`, `check`, `build`, `test`)
- [x] 確認 `Dockerfile`、`Dockerfile_live`、`build.bat`、`build.ps1`、`build.sh` 無寫死 `src/` 內特定路徑

#### D. Gate 執行結果

- [x] `cargo check` 通過
- [x] `cargo build` 通過
- [x] `cargo fmt --all -- --check` 通過
- [x] 無跨層違規引用（`grep` 檢查確認無異常）

建議填寫欄位：

- `fmt 結果`：成功
- `check 結果`：成功
- `build 結果`：成功

#### E. 中斷交接資訊（Resume）

- `Last Update Time`：2026-05-08
- `Stopped At`：全部完成，準備 commit 與 push
- `Next Action (one step)`：git commit
- `Known Risks`：無

#### F. 合併前確認

- [x] 與該 Phase 無關的變更已排除
- [x] Commit 訊息符合 `stage-x` 規則
- [x] PR 說明已附 Checklist 摘要與 Gate 結果
- [x] 回滾點（tag/SHA）已再次確認
