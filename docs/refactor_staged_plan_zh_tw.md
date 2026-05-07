# stock_crawler 重構企劃（分階段、可編譯前進）

更新日期：2026-05-07

## 1. 掃描結果摘要（現況）

本次已掃描整個專案（重點為 `src/` Rust 程式碼）：

- Rust 檔案總數：`179`
- 主要高體量模組：
- `crawler/`：`67` 檔，約 `8948` 行
- `database/`：`36` 檔，約 `7289` 行
- `event/`：`12` 檔，約 `2789` 行
- `rpc/`：`12` 檔，約 `2618` 行
- `util/`：`10` 檔，約 `2273` 行
- `backfill/`：`19` 檔，約 `2220` 行
- `src` 根目錄仍有多個核心檔案：`config.rs`、`declare.rs`、`scheduler.rs`、`manual_backfill.rs`、`main.rs`

結論：目前屬於「技術分層 + 歷史成長混雜」結構，`src/` 根目錄過重，`crawler/database` 體量很大，適合採用**先穩定、再搬移、每階段編譯鎖門**的重構策略。

## 2. 重構目標

目標不是一次大翻修，而是建立可持續演進的目錄結構，同時降低搬移風險：

- 降低 `src/` 根目錄負擔，集中入口責任。
- 明確分離：應用流程、基礎設施、外部介面、共用核心。
- 每個階段完成後都必須可編譯，才進入下一階段。

建議目標結構：

```text
src/
├─ main.rs
├─ core/          # config / declare / util / logging
├─ app/           # scheduler / backfill / manual_backfill / event / calculation
├─ infra/         # crawler / database / cache / nosql
└─ interfaces/    # rpc / web / bot
```

## 3. 執行原則（必要）

1. 每次只做一個階段，避免跨階段混改。
2. 每次搬移先做「檔案移動 + mod 宣告 + use 路徑修正」，不先改商業邏輯。
3. 階段結束必跑編譯關卡；任一關失敗即回修，不得進入下一階段。
4. **編譯成功是唯一硬性要求**：`cargo check` 與 `cargo build` 必須通過。`cargo test` 中涉及 Redis、PostgreSQL、外部 API 等實際連線的測試，若因環境未就緒而失敗，屬於預期中的情況，可記錄後略過，不阻擋下一階段。
5. 每階段的開發流程規範：
   - 必須基於最新的 `main` 建立新的短分支（例如 `refactor/stage-1-core`）。
   - 該階段開發完成且本地編譯（`cargo check` 與 `cargo build`）通過後，必須 Push 到遠端並發起 PR。
   - 確認 PR 沒問題並**合併 (Merge) 回 `main`** 後，才能進行下一階段。
   - 下一階段的分支，必須先切換回 `main` 並拉取最新進度，再從乾淨的 `main` 建立新的分支。
6. 優先使用 Rust-analyzer 的 Rename/Move 支援調整路徑，避免手動替換漏網。
7. 若 Windows 或低記憶體環境出現 `Error: Not enough memory resources are available to complete this operation. (os error 14)`，Gate 指令改用低併發模式：`cargo check -j 1`、`cargo build -j 1`。此錯誤通常代表同時啟動過多編譯程序或 linker 壓力過高，不代表重構本身一定有語法錯誤。

## 4. 分階段重構計畫

### Phase 0：輕量基線確認（不改功能） ✅ [已完成]

目的：建立重構前的最小可比較基準，避免後續無法判斷錯誤是既有問題或重構引入。

工作項目：

- 記錄目前分支與起始 commit SHA。
- 建立 `pre-stage-1` tag 或明確記錄可回溯的起始 SHA。
- `cargo check`
- `cargo build`
- 低記憶體環境可改跑：`cargo check -j 1`、`cargo build -j 1`
- 嘗試執行 `cargo fmt --all -- --check`，若失敗只記錄，不阻擋 Phase 1。
- 嘗試執行 `cargo test -- --nocapture`，若因環境依賴或既有測試問題失敗，只記錄，不阻擋 Phase 1。
- 記錄目前已知 warning、既有 failure 與可忽略項目（例如舊模組未使用函式）。

通關條件（Gate）：

- `cargo check` 已執行並記錄結果。
- `cargo build` 已執行並記錄結果。
- 若使用低併發模式，需記錄實際指令（例如 `cargo check -j 1`、`cargo build -j 1`）。
- 已建立 `pre-stage-1` tag，或已記錄起始 commit SHA。
- `fmt/test` 若失敗，需有明確紀錄與原因判斷；不作為 Phase 1 的阻擋條件。

---

### Phase 1：抽出 core（低風險先行） ✅ [已完成]

搬移範圍：

- `config.rs` -> `core/config.rs`
- `declare.rs` -> `core/declare.rs`
- `util/` -> `core/util/`
- `logging/` -> `core/logging/`
- 新增 `core/mod.rs`

同步調整：

- `main.rs` 與所有 `use crate::...` 改為 `crate::core::...`
- 保持 API 名稱不變，先不重命名型別與函式。
- ⚠️ `config.rs` 內部有 `use crate::logging`（`from_env` 與 `override_with_env` 多處呼叫 `logging::error_file_async`），兩者同時搬入 `core/` 後需改為 `super::logging::` 或 `crate::core::logging::`。
- ⚠️ `logging/` 與 `util/` 內部也有自引用路徑，不能只修外部使用端：
- `logging/mod.rs`：`crate::logging::rotate::Rotate`、`crate::util::atomic::decrement_atomic_usize`
- `logging/rotate.rs`：`crate::logging`
- `util/datetime.rs`：`crate::declare::Quarter`、`crate::logging`
- `util/http/element.rs`：`crate::util::text`
- `util/http/mod.rs`：`crate::logging::LoggerRuntimeStatus`、`crate::logging`
- 建議 Phase 1 完成後執行 `rg -n "crate::(config|declare|logging|util)" src`，確認沒有漏掉應改為 `crate::core::...` 的引用。

通關條件（Gate）：

- `cargo check`
- `cargo build`

---

### Phase 2：抽出 interfaces（外部入口集中） ⏳ [準備中]

搬移範圍：

- `rpc/` -> `interfaces/rpc/`
- `web/` -> `interfaces/web/`
- `bot/` -> `interfaces/bot/`
- 新增 `interfaces/mod.rs`

同步調整：

- 啟動點改為 `crate::interfaces::rpc::...`、`crate::interfaces::web::...`
- 修正 `rpc` 內部 client/server 與 proto 產物路徑引用。
- ⚠️ **`build.rs` 必須同步修改**：`build.rs` 內有硬編碼 `static OUT_DIR: &str = "src/rpc";`，`tonic_prost_build` 的 proto 產碼輸出到此目錄。搬移後必須改為 `"src/interfaces/rpc"`，否則下次修改 `.proto` 重新建置時產碼會輸出到錯誤位置。
- ⚠️ `build.rs` 檔案註解也寫明產生到 `src/rpc/`，搬移時需一併更新，避免後續維護者誤判。
- `rpc/mod.rs` 使用 `include!("stock.rs")`、`include!("basic.rs")`、`include!("control.rs")`、`include!("manual_backfill.rs")`，搬移 `rpc/` 目錄時這些被 include 的產碼檔必須一起移動到 `interfaces/rpc/`，不可只搬 `mod.rs`。

通關條件（Gate）：

- `cargo check`
- `cargo build`

輔助驗證（有環境才執行）：

- 啟動驗證：服務可啟動至初始化完成（至少通過本地啟動一次）

---

### Phase 3：抽出 app（流程編排集中）

搬移範圍：

- `scheduler.rs` -> `app/scheduler.rs`
- `manual_backfill.rs` -> `app/manual_backfill.rs`
- `backfill/` -> `app/backfill/`
- `event/` -> `app/event/`
- `calculation/` -> `app/calculation/`
- 新增 `app/mod.rs`

> **為何 `event/` 與 `calculation/` 歸入 `app/` 而非 `domain/`？**
> 經程式碼比對，`event::taiwan_stock::closing` 重度依賴 `backfill`、`bot`、`crawler`、`database::table::*`；
> `event::trace::stock_price` 依賴 `nosql`、`crawler::twse`、`database::table::trace`；
> `calculation::money_history` 依賴 `database`。
> 這些模組本質上是「業務編排 / 應用用例」，放入 `domain/` 會造成 `domain → infra` 的反向依賴，違反分層原則。

同步調整：

- `main.rs` 改呼叫 `crate::app::scheduler::start(...)`
- `backfill` / `event` / `calculation` 對 `crawler/database/cache` 引用暫維持舊路徑（下一階段再搬 infra）。
- `event` / `calculation` / `scheduler` / `interfaces` 中若引用 `crate::backfill`、`crate::event`、`crate::calculation`、`crate::scheduler`，需改為 `crate::app::...`。
- 已知引用點：`event/taiwan_stock/closing.rs` 有 `crate::event::trace::price_tasks::stop_price_tasks()`，搬入 `app/event` 後需改為 `crate::app::event::trace::price_tasks::stop_price_tasks()` 或改用相對路徑。
- ⚠️ `manual_backfill.rs` 在 `main.rs` 中以 `#[cfg(test)] mod manual_backfill;` 條件編譯宣告，搬入 `app/` 後需在 `app/mod.rs` 中同樣加上 `#[cfg(test)] mod manual_backfill;`，並從 `main.rs` 移除原宣告。
- `manual_backfill.rs` 內的測試註解仍寫 `cargo test manual_backfill::...`，搬移後需同步改成 `cargo test app::manual_backfill::...` 或改為只用測試函式名稱篩選。

通關條件（Gate）：

- `cargo check`
- `cargo build`

輔助驗證（有環境才執行）：

- 手動觸發一個 backfill 流程（最小案例）可跑到結束

---

### Phase 4：抽出 infra（大體量高風險，拆為子步驟）

此階段搬移量最大（4 個模組、合計超過 120 檔），為降低風險拆為三個子步驟，每個子步驟獨立跑 Gate。

#### Phase 4a：cache + nosql → infra（低風險先行）

搬移範圍：

- `cache/` -> `infra/cache/`
- `nosql/` -> `infra/nosql/`
- 新增 `infra/mod.rs`

同步調整：

- 全域路徑替換：`crate::cache::` -> `crate::infra::cache::`、`crate::nosql::` -> `crate::infra::nosql::`
- 注意 `cache/mod.rs` 有 `pub use` 再匯出（如 `pub use share::SHARE`），使用端若以 `crate::cache::SHARE` 引用，需一併替換。
- `cache/share.rs` 目前引用 `crate::crawler::share as crawler_share`。Phase 4a 時 `crawler/` 尚未搬移，此引用暫時維持 `crate::crawler::...`，等 Phase 4c 再改為 `crate::infra::crawler::...`。

通關條件（Gate）：`cargo check` + `cargo build`

#### Phase 4b：database → infra

搬移範圍：

- `database/` -> `infra/database/`

同步調整：

- 全域路徑替換：`crate::database::` -> `crate::infra::database::`
- `database/mod.rs` 內 `use crate::config` 已在 Phase 1 改為 `crate::core::config`，確認路徑正確即可。
- `database/table/*` 中有多處 `use crate::database`、`crate::database::table::*` 自引用，搬移後需改為 `crate::infra::database` 或使用相對路徑。
- `database/table/*` 中也有引用 `crate::crawler`、`crate::cache`、`crate::logging`、`crate::util`，需依 Phase 1/4a 的結果同步確認，不能只替換 `crate::database`。

通關條件（Gate）：`cargo check` + `cargo build`

#### Phase 4c：crawler → infra（最大量）

搬移範圍：

- `crawler/` -> `infra/crawler/`

同步調整：

- 全域路徑替換：`crate::crawler::` -> `crate::infra::crawler::`
- `crawler/` 內有 5 處引用 `crate::cache::SHARE`（如 `goodinfo/dividend.rs`、`twse/eps.rs` 等），已在 4a 改過路徑，此處需改為 `crate::infra::cache::SHARE`（同層內引用）。
- `crawler/` 內也有多處自引用 `crate::crawler::...`（例如 `price_tasks.rs`、`share.rs`、`yahoo/price/cache.rs`、`twse/*`），搬移後都需改成 `crate::infra::crawler::...` 或相對路徑。
- 先完成「可編譯搬移」，不在本階段做 database schema/SQL 重構。

通關條件（Gate）：

- `cargo check`
- `cargo build`

輔助驗證（有環境才執行）：

- 至少一次完整啟動 + gRPC 自測 + Redis ping 正常

---

### Phase 5：infra/database 內部領域化分組

> `event/` 與 `calculation/` 已在 Phase 3 歸入 `app/`，本階段專注於 `infra/database/table/` 的內部優化。

工作項目：

針對 `infra/database/table/` 目前扁平的 36 檔結構，依業務領域建立子目錄（先搬目錄，暫不改 SQL 行為）：

- `stock/`：股票主檔、交易所、市場、產業、持股明細等（含既有的 `stock/` 子目錄）
- `quote/`：日報價（`daily_quote/`）、最後報價（`last_daily_quotes`）、歷史報價（`quote_history_record`）與統計（`daily_stock_price_stats`）
- `dividend/`：股利主檔（含既有的 `dividend/`）、配發日、明細延伸資料（`dividend_record_detail/`、`dividend_record_detail_more`）
- `financial/`：財報（`financial_statement`）與估值（`estimate`）
- `money_flow/`：資金流向與法人資料（`daily_money_history*`）
- `revenue/` 或 `financial/revenue.rs`：營收資料（`revenue`）需在 Phase 5 開始前決定歸屬，建議先放 `financial/`，避免新增過細目錄。
- `ops/` 或保留根層：`config`、`trace`、`yield_rank` 這類橫切或輔助表，不應硬塞到股票/報價/股利分類；若歸屬不明，Phase 5 可先保留在 `table/` 根層，等後續再拆。
- `table/mod.rs` 必須同步維護再匯出策略，避免一次移動後造成大量 `crate::infra::database::table::<name>` 使用端失效。

Phase 5 建議拆成兩小步：

- `Phase 5a`：只建立領域目錄與移動低風險檔案（`quote`、`money_flow`），每搬一組跑一次 `cargo check`。
- `Phase 5b`：處理 `stock/dividend/financial` 與不明確歸屬檔案，必要時透過 `pub use` 暫時保留舊路徑相容。

通關條件（Gate）：

- `cargo check`
- `cargo build`

輔助驗證（有環境才執行）：

- 核心事件與計算相關流程 smoke test 通過

---

### Phase 6：收尾與規範固化

工作項目：

- 更新 `README.md` 架構圖與開發者導覽。
- 新增「新模組放置規範」到 `docs/architecture.md`（建議新增）。
- 補 CI 檢查（至少 `fmt + check + build`）。
- 檢查 `Dockerfile`、`Dockerfile_live`、`build.bat`、`build.ps1`、`build.sh` 是否有寫死 `src/` 內的特定路徑（如 `COPY` 指令）。
- 清理已棄用的歷史遺留：
  - 刪除 `Rocket.toml`（僅 22 bytes，Rocket 早已棄用改為 Axum）。
  - 清除 `main.rs` 中被註解掉的 Rocket 程式碼區塊與底部的公式註解。

通關條件（Gate）：

- CI 綠燈
- 無跨層違規引用（透過 code review 規則檢查）

## 5. 每階段標準作業流程（SOP）

每一階段都使用同一流程：

1. 建分支：`git switch -c refactor/stage-x-<name>`
2. 移檔與 `mod.rs` 更新。
3. 全域修正 `use` 路徑（請見下方「路徑替換最佳實踐」）。
4. 執行硬性 Gate：
5. `cargo check`（低記憶體環境使用 `cargo check -j 1`）
6. `cargo build`（低記憶體環境使用 `cargo build -j 1`）
7. 執行輔助檢查（非硬性 Gate）：
8. `cargo fmt --all -- --check` 或 `cargo fmt --all`
9. （可行時）`cargo test`——涉及 Redis/PostgreSQL/外部 API 連線的測試若因環境問題失敗可略過，只要編譯成功即可
10. 提交 commit（訊息含 `stage-x`）
11. 合併後才開始下一階段

### 路徑替換最佳實踐（取代手動腳本）

在數萬行程式碼的專案中，使用 Regex（如 `sed` 或 PowerShell 替換）容易出錯，尤其是面對多重 import（`use crate::{crawler, database};`）或再匯出（`pub use`）時。

**推薦的 AI 與 IDE 協作方案：**

1. **優先使用 Rust-analyzer (IDE 重構工具)**：
   - 在 VSCode/IntelliJ 中，對資料夾或檔案點擊 **Rename / Move**。Rust-analyzer 會自動解析語法樹，並幫你把專案中所有引用該模組的路徑（包含多重 import）一次改對。
2. **使用編譯器引導的 AI 替換**：
   - 搬移檔案後，直接執行 `cargo check`。
   - 將 `cargo check` 噴出的大量「unresolved import」錯誤訊息直接餵給 AI（例如目前的對話視窗），讓 AI 分析錯誤後以 `apply_patch` 套用精準修改。
   - 這種做法比 Regex 安全，且 AI 懂得處理多重 import 的拆解。
3. **最後防線 `cargo clippy`**：
   - 替換完成後，除了 `cargo check`，可執行 `cargo clippy` 抓出潛在的未使用 import 或多餘路徑；`clippy` 建議作為輔助檢查，不作為每階段硬性 Gate。

## 6. 風險與對策

- 風險：Phase 4 (`infra`) 改動面最大，容易漏改路徑。
- 對策：已拆為 4a/4b/4c 子步驟，先搬 `cache/nosql`，再搬 `database`，最後搬 `crawler`，每小步都跑 `cargo check`。

- 風險：`main.rs` 入口同時依賴多層，階段交界容易衝突。
- 對策：保留 `main.rs` 為唯一組裝點，不在中途拆成多入口。

- 風險：重構期間功能驗證不足。
- 對策：每階段至少執行一次「啟動 + 最小流程 smoke test」。

- 風險：`build.rs` proto 產碼路徑寫死為 `src/rpc`，Phase 2 搬移後若漏改，下次改 proto 會產碼到錯誤位置。
- 對策：Phase 2 Checklist 中明確列入 `build.rs` 的 `OUT_DIR` 路徑更新。

- 風險：`use crate::{crawler, database, ...}` 多重 import 語法無法被簡單字串替換覆蓋。
- 對策：放棄 Regex，改用 Rust-analyzer 內建的 Move/Rename 功能，或將 `cargo check` 錯誤餵給 AI 進行精確替換。

- 風險：Windows 或資源受限環境在 `cargo build` / `cargo test` 時出現 `os error 14` 記憶體資源不足。
- 對策：先停止其他大型程序，改用 `cargo check -j 1` 與 `cargo build -j 1` 重新驗證；若仍失敗，再記錄當下階段、最後一個搬移項目與完整錯誤訊息，不要直接進入下一階段。

## 7. 建議的第一步（立即可做）

先完成輕量 `Phase 0`，確認基線後再啟動 `Phase 1`，原因：

- `Phase 0` 只建立可比較基準，不修改程式碼。
- 基線確認後再搬 `core`，後續若出錯才能分辨是既有問題或重構引入。

## 8. 中長期補強（可在 Phase 6 之後）

- 導入 Cargo Workspace，將 `core`、`app`、`infra` 逐步拆成多 crate，以編譯邊界強化分層約束。
- 建議拆分順序：先 `core`、再 `app`、最後 `infra`（改動面最大）。

## 9. 各階段預估工時（供排程）

以下為「單人主責、熟悉專案」的粗估，實際仍需依當前分支衝突與測試覆蓋率調整：

- `Phase 0`：`1` 到 `2` 小時
- `Phase 1 (core)`：`1` 到 `1.5` 天
- `Phase 2 (interfaces)`：`1` 到 `2` 天
- `Phase 3 (app + event + calculation)`：`1.5` 到 `2.5` 天
- `Phase 4a/4b/4c (infra)`：`2` 到 `4` 天（最高風險）
- `Phase 5 (database table 分組)`：`1` 到 `2` 天
- `Phase 6 (收尾/CI/文件/清理)`：`0.5` 到 `1` 天

總工期粗估：`7.5` 到 `13.5` 個工作天。

## 10. 回滾策略模板（每階段必備）

每個 Phase 開始前，先建立對應分支與回滾點：

1. 建立階段分支：`refactor/stage-x-<name>`
2. 建立回滾 tag：`git tag pre-stage-x`
3. 階段內至少切 2 到 4 個小 commit（避免單一超大 commit）
4. Gate 失敗時執行回滾策略：

- 輕度失敗（可快速修復）：在同分支直接修到 Gate 綠燈
- 中度失敗（改動面過大）：`git reset --hard pre-stage-x`（僅限該重構分支自行使用）
- 重度失敗（影響多人整合）：關閉當前 PR，重新切分更小子階段

PR 描述建議固定包含：

- 變更範圍（搬移哪些模組）
- 路徑替換規則（例如 `crate::crawler` -> `crate::infra::crawler`）
- Gate 結果截圖或文字紀錄（`fmt/check/build/test`）
- 回滾點（tag 或 commit SHA）

## 11. Phase Checklist（中斷可續作）

目的：任何人中斷後回來，都能從目前進度接手，不需整個 Phase 重來。

使用方式：

- 每個 Phase 開始時複製一份模板，存成 `docs/checklists/phase-x-checklist.md`。
- 執行過程持續更新勾選與紀錄欄位。
- PR 送審前，Checklist 必須完整。

---

### Phase 通用 Checklist 模板

#### A. 基本資訊

- [ ] Phase 編號與名稱已填寫
- [ ] 負責人已填寫
- [ ] 分支名稱已填寫（`refactor/stage-x-<name>`）
- [ ] 起始 commit SHA 已填寫
- [ ] 回滾 tag 已建立（例如 `pre-stage-x`）

建議填寫欄位：

- `Phase`：
- `Owner`：
- `Branch`：
- `Start SHA`：
- `Rollback Tag`：
- `預估完成日`：

#### B. 搬移清單（Move List）

- [ ] 目標搬移檔案/目錄清單已列出
- [ ] 已完成搬移項目已逐條標記
- [ ] 目標 `mod.rs` 宣告已同步新增
- [ ] `main.rs` 中的舊 `pub mod xxx;` 宣告已移除（每搬一個模組就要同步刪除）
- [ ] 若有 `#[cfg(test)]` 條件編譯模組（如 `manual_backfill`），目標 `mod.rs` 已加上相同的 `#[cfg(test)]` 屬性
- [ ] 尚未搬移項目有明確原因註記

建議填寫欄位：

- `已完成搬移`：
- `待搬移`：
- `main.rs 已移除的 mod 宣告`：
- `阻塞原因`：

#### C. 路徑修正清單（use/module path）

- [ ] 全域替換規則已記錄（例：`crate::crawler` -> `crate::infra::crawler`）
- [ ] 主要入口模組（`main.rs`、`scheduler`、`rpc/web`）已檢查
- [ ] `use crate::{A, B, ...}` 多重 import 語法已手動檢查並拆開修正（無法被批次替換覆蓋）
- [ ] `pub use` 再匯出路徑已確認更新（如 `cache/mod.rs` 的 `pub use share::SHARE`）
- [ ] （若適用）`build.rs` 或其他外部設定檔中的硬編碼路徑已同步修改（如 `OUT_DIR`）
- [ ] 編譯錯誤修正紀錄已保留（避免重踩）

建議填寫欄位：

- `替換規則`：
- `已修正錯誤類型`：
- `仍待修正錯誤`：
- `外部檔案變更`：（如 `build.rs OUT_DIR`、`Dockerfile`）

#### D. Gate 執行結果

- [ ] `cargo check` 通過
- [ ] `cargo build` 通過
- [ ] 若曾出現 `os error 14`，已改用 `-j 1` 低併發 Gate 重新確認並記錄結果
- [ ] （輔助）`cargo fmt --all -- --check` 已執行並記錄結果；若未通過，不阻擋下一階段
- [ ] （如適用）`cargo test` 或 smoke test 通過（僅需編譯期測試通過；涉及 Redis/PostgreSQL/外部 API 實際連線的測試若因環境未就緒而失敗，記錄後可略過）
- [ ] （如適用）服務可啟動至初始化完成（Phase 2、4c、6 的輔助驗證；若無對應環境可跳過並註記）
- [ ] （如適用）gRPC 自測 + Redis ping 正常（Phase 4c 的輔助驗證；若無對應環境可跳過並註記）
- [ ] 若為 `Phase 0`，`fmt/test` 失敗已記錄原因，且確認不阻擋 `Phase 1`

建議填寫欄位：

- `fmt 結果`：
- `check 結果`：
- `build 結果`：
- `test/smoke 結果`：
- `啟動驗證結果`：（附上啟動 log 或截圖）
- `失敗時最後錯誤訊息`：

#### E. 中斷交接資訊（Resume）

- [ ] 最後工作時間已記錄
- [ ] 最後停在第幾步已記錄
- [ ] 下一步「單一可執行動作」已寫明
- [ ] 風險與注意事項已更新

建議填寫欄位：

- `Last Update Time`：
- `Stopped At`：
- `Next Action (one step)`：
- `Known Risks`：

#### F. 合併前確認

- [ ] 與該 Phase 無關的變更已排除
- [ ] `main.rs` 已無殘留的舊 `pub mod` 宣告（已搬移的模組不應再出現在 `main.rs`）
- [ ] Commit 訊息符合 `stage-x` 規則
- [ ] PR 說明已附 Checklist 摘要與 Gate 結果
- [ ] 回滾點（tag/SHA）已再次確認
- [ ] （若為 Phase 4c/6）服務啟動驗證結果已附在 PR

---

### 建議的最小續作流程（中斷後）

1. 先讀該 Phase checklist 的 `Stopped At` 與 `Next Action`。
2. 執行 `cargo check`，確認當前基線狀態。
3. 只做 `Next Action` 這一小步，完成後立即更新 checklist。
4. 每完成一批搬移就再跑一次 `cargo check`，避免累積錯誤。
