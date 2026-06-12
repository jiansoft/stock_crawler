# stock_crawler DDD 重構與優化後續計畫 (Phase 18 - 20)

更新日期：2026-06-12

本計畫依據 Phase 1 ~ 17 的成果，針對「爬蟲與資料庫解耦 (方向二)」、「清理 `app.json` 設定 (方向四)」、與「測試架構升級 (方向三)」制定詳細的執行步驟。

---

## 計畫總覽

```text
Phase 18 (爬蟲解耦) ──> Phase 19 (設定清理) ──> Phase 20 (測試與 CI 升級)
  ├─ 18a: QFII DTO 隔離與刪除舊 Table  ├─ 19a: 清理 app.json          ├─ 20a: 引入 3+ 核心爬蟲 Fixture 測試
  └─ 18b: ETF/ISIN 解耦市場快取與 Table └─ 19b: 清理 config.rs         └─ 20b: 活化 CI 資料庫整合測試
```

---

## Phase 18：解耦 QFII/ETF 爬蟲與資料庫 Table Model (方向二) ✅ [已完成]

### 18a：QFII 爬蟲與 Table Model 隔離
目的：使外資持股 (QFII) 爬蟲不依賴資料庫 Table 結構，並清理已無用之 legacy table query。

工作項目：
- 於 [src/infra/crawler/share.rs](file:///D:/Project/Eddie/stock_rust/src/infra/crawler/share.rs) 定義 `QfiiDto`：
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct QfiiDto {
      pub stock_symbol: String,
      pub issued_share: i64,
      pub shares_held: i64,
      pub share_holding_percentage: rust_decimal::Decimal,
  }
  ```
- 修改 [src/infra/crawler/twse/qualified_foreign_institutional_investor/listed.rs](file:///D:/Project/Eddie/stock_rust/src/infra/crawler/twse/qualified_foreign_institutional_investor/listed.rs) 與 [over_the_counter.rs](file:///D:/Project/Eddie/stock_rust/src/infra/crawler/twse/qualified_foreign_institutional_investor/over_the_counter.rs)：
  - 移除對 `table::stock::extension::qualified_foreign_institutional_investor::QualifiedForeignInstitutionalInvestor` 的引用。
  - 將 `visit` 的傳回型別改為 `Result<Vec<QfiiDto>>`。
  - 將解析原始 API / HTML 的邏輯直接實作並對應到 `QfiiDto`。
- 修改 [src/app/backfill/acl.rs](file:///D:/Project/Eddie/stock_rust/src/app/backfill/acl.rs)：
  - 移除對舊 Table `QualifiedForeignInstitutionalInvestor` 的引用。
  - 修改 `QfiiAclMapper::from_qfii`，使其接收 `&QfiiDto` 並轉譯為 `UpdateQfiiCommand`。
- 修改 [src/app/backfill/qualified_foreign_institutional_investor.rs](file:///D:/Project/Eddie/stock_rust/src/app/backfill/qualified_foreign_institutional_investor.rs)：
  - 調整變數型別，確保以 `QfiiDto` 傳遞資料。
- 刪除舊 Table extension [src/infra/database/table/stock/extension/qualified_foreign_institutional_investor.rs](file:///D:/Project/Eddie/stock_rust/src/infra/database/table/stock/extension/qualified_foreign_institutional_investor.rs) 及其在 [extension/mod.rs](file:///D:/Project/Eddie/stock_rust/src/infra/database/table/stock/extension/mod.rs) 的宣告（該 table 級 update 方法已無使用）。

通關條件 (Gate)：
- `cargo check`
- `cargo test app::backfill::qualified_foreign_institutional_investor::tests` 通過。

---

### 18b：ETF / ISIN 爬蟲解耦與快取移除
目的：使 ETF 與 ISIN 爬蟲不再依賴 `table::stock_exchange_market::StockExchangeMarket` 與運行時之 `SHARE` 記憶體快取。

工作項目：
- 於 [src/infra/crawler/share.rs](file:///D:/Project/Eddie/stock_rust/src/infra/crawler/share.rs) 修改 `EtfInfo`：
  - 將 `exchange_market: table::stock_exchange_market::StockExchangeMarket` 改為 `market: StockExchangeMarket` (使用 [src/core/declare.rs](file:///D:/Project/Eddie/stock_rust/src/core/declare.rs) 的 enum)。
- 於 [src/infra/crawler/twse/international_securities_identification_number.rs](file:///D:/Project/Eddie/stock_rust/src/infra/crawler/twse/international_securities_identification_number.rs) 修改 `InternationalSecuritiesIdentificationNumber`：
  - 將 `exchange_market` 欄位改為 `market: StockExchangeMarket`。
- 修改 [src/infra/crawler/twse/etf.rs](file:///D:/Project/Eddie/stock_rust/src/infra/crawler/twse/etf.rs)、[src/infra/crawler/tpex/etf.rs](file:///D:/Project/Eddie/stock_rust/src/infra/crawler/tpex/etf.rs) 與 [isin.rs](file:///D:/Project/Eddie/stock_rust/src/infra/crawler/twse/international_securities_identification_number.rs)：
  - 移除對 `SHARE.stock_exchange_markets` 唯讀快取的依賴。
  - 直接在爬蟲中賦值 `StockExchangeMarket::Listed` (TWSE) 或 `StockExchangeMarket::OverTheCounter` (TPEX)。
  - ISIN 爬蟲移除 `SHARE.get_industry_id(&industry)` 依賴，直接返回原始 `industry` 字串與 `market` enum。
- 於 [src/app/backfill/acl.rs](file:///D:/Project/Eddie/stock_rust/src/app/backfill/acl.rs) 重構 `IsinAclMapper` 與 `EtfAclMapper`：
  - 在防腐層 (ACL) 中，透過 `SHARE` 快取查詢對應的 `market_id` 與 `industry_id`，轉譯為 `RegisterStockCommand`。
- 效益：爬蟲模組退化為純粹的外部資料 Parser，不再需要依賴快取載入，使其能被獨立且快速地進行單元測試。

通關條件 (Gate)：
- `cargo check`
- `cargo test app::backfill::etf::tests` 與 `cargo test app::backfill::isin::tests` 通過。

---

## Phase 19：清理已棄用的設定欄位 (方向四 - 局部) ⏳ [待啟動]

### 19a：清理 `app.json`
- 從 [app.json](file:///D:/Project/Eddie/stock_rust/app.json) 中刪除以下不再使用的區塊：
  - `"afraid"` (DNS 服務)
  - `"dyny"` (Dynu DNS 服務，且有拼寫錯誤)
  - `"noip"` (No-IP DNS 服務)

### 19b：清理 `config.rs` 解析代碼
- 修改 [src/core/config.rs](file:///D:/Project/Eddie/stock_rust/src/core/config.rs)：
  - 移除 `Afraid`、`Dynu` (或 `Dyny`)、`Noip` 相關結構體。
  - 移除 `Config` 結構體中的 `afraid`、`dyny`、`noip` 欄位。
  - 移除環境變數映射常數（如 `AFRAID_TOKEN`、`DYNU_USERNAME`、`DYNU_PASSWORD` 等）。
  - 移除與這些 DNS 設定相關的預設值填充和環境變數覆蓋邏輯。
- 確認主程式啟動與測試無異常。

通關條件 (Gate)：
- `cargo check`
- `cargo build`

---

## Phase 20：升級自動化測試架構與 CI/CD 整合測試 (方向三) ⏳ [待啟動]

### 20a：核心爬蟲解析單元測試 Fixture 化
目的：為核心爬蟲編寫不依賴真實網路的單元測試，使 CI 能夠自動執行解析測試。

工作項目：
- 挑選 3 個核心爬蟲進行 Fixture 重構：
  - [src/infra/crawler/twse/quote.rs](file:///D:/Project/Eddie/stock_rust/src/infra/crawler/twse/quote.rs) (上市報價)
  - [src/infra/crawler/tpex/quote.rs](file:///D:/Project/Eddie/stock_rust/src/infra/crawler/tpex/quote.rs) (上櫃報價)
  - [src/infra/crawler/yahoo/dividend.rs](file:///D:/Project/Eddie/stock_rust/src/infra/crawler/yahoo/dividend.rs) (Yahoo 股利)
- 於 `tests/fixtures/` 或測試模組內部準備 JSON/HTML 的真實數據片段（Mock Data）。
- 重構爬蟲，提取出**純解析函數**（例如 `parse_quote_json(raw_json: &str) -> Result<Vec<DailyQuoteDto>>`）。
- 針對解析函數編寫單元測試，載入 Fixture 資料並驗證欄位轉換（如 Decimal 轉換、日期轉換等）。移除 these 解析測試上的 `#[ignore]` 標籤。

通關條件 (Gate)：
- `cargo test` 執行新加入的單元測試，且均無 `#[ignore]` 並全數通過。

---

### 20b：活化 CI 階段的資料庫整合測試
目的：利用 GitHub Actions 現有的 Postgres 與 Redis 服務，自動執行資料庫/快取相關的整合測試，不再無條件 `#[ignore]`。

工作項目：
- 盤點與分類現有 `#[ignore]` 的測試（如 [phase-0-5-ignored-tests-exemptions.md](file:///D:/Project/Eddie/stock_rust/docs/checklists/phase-0-5-ignored-tests-exemptions.md) 所述）：
  - **Type A**：純資料庫/快取/Repository 邏輯測試（如 `PgQuoteRepository` 寫入與讀取、`PgStockRepository` 存取）。
  - **Type B**：必須連線真實外部 API 的爬蟲與回補測試。
- 對於 **Type A** 測試：
  - 移除 `#[ignore]` 標籤，或是將其移動到專門的整合測試模組（如 `#[cfg(feature = "integration-test")]`，或是在測試函式中檢測是否能成功連線 DB 後動態略過，而非靜態 `#[ignore]`）。
- 修改 CI 腳本 [.github/workflows/rust.yml](file:///D:/Project/Eddie/stock_rust/.github/workflows/rust.yml)：
  - 目前 CI 已經在 test job 中拉起了 postgres 與 redis。
  - 將 `cargo test` 命令調整為包含 Type A 整合測試（例如使用 `--` 指定篩選，或運行特定的 Repository 測試）。
  - 確保當 Repository SQL 語法錯誤時，CI 能夠第一時間亮紅燈警告。

通關條件 (Gate)：
- GitHub Actions 整合測試執行成功。
- 在本機有啟動 Postgres/Redis 的情況下，核心 Repository 測試均能通過。

---

## 執行 SOP 與回滾機制
1. 建立短分支：`refactor/ddd-phase-x`
2. 每次只執行一個子步驟，執行完立即進行 `cargo check` 與 `cargo test`。
3. 若測試不通過或破壞編譯，使用 `git checkout -- <file>` 回滾。
4. PR 審查合併後，方可建立下一個 Phase 的分支。
