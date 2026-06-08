# Phase 9: 股利與持股領域 (Dividend & Holding Domain) DDD 化 Checklist

## A. 基本資訊
- [x] Phase 編號與名稱：Phase 9: 股利與持股領域 DDD 化
- [x] 負責人：Antigravity
- [x] 分支名稱：`refactor/ddd-phase-9`
- [x] 起始 commit SHA：1bbd8e13d105ad36cbfbde1f278eb5dfc0bb366b
- [x] 回滾 tag 已建立：pre-stage-9

## B. 設計與定義 (Domain & Repository Definition)
- [x] **建立 `src/domain/dividend/` 目錄**
  - [x] 建立 `src/domain/dividend/mod.rs` 宣告子模組
  - [x] 建立 `src/domain/dividend/entity.rs` 定義 `Dividend` 領域實體與商業邏輯
  - [x] 建立 `src/domain/dividend/repository.rs` 定義 `DividendRepository` 倉儲介面
- [x] **建立 `src/domain/portfolio/` 目錄**
  - [x] 建立 `src/domain/portfolio/mod.rs` 宣告子模組
  - [x] 建立 `src/domain/portfolio/entity.rs` 定義 `StockOwnershipDetail` (持股明細) 及 `ReceivedDividend`、`ReceivedDividendItem` (已領股利總表/明細) 領域實體與商業邏輯
  - [x] 建立 `src/domain/portfolio/repository.rs` 定義 `PortfolioRepository` 倉儲介面
- [x] **更新 `src/domain/mod.rs`**
  - [x] 匯出 `pub mod dividend;` 與 `pub mod portfolio;`

## C. 基礎設施實現 (Infrastructure Implementation)
- [x] **實作 `PgDividendRepository`**
  - [x] 於 `src/infra/database/repository/dividend.rs` 實現 `DividendRepository` 介面
- [x] **實作 `PgPortfolioRepository`**
  - [x] 於 `src/infra/database/repository/portfolio.rs` 實現 `PortfolioRepository` 介面
- [x] **更新 `src/infra/database/repository/mod.rs`**
  - [x] 匯出新的 Repository 模組

## D. 應用層重構 (Application Layer Refactoring)
- [x] **重構 `src/app/calculation/dividend_record.rs`**
  - [x] 改為注入與呼叫 `DividendRepository` 與 `PortfolioRepository`
  - [x] 將除權息資格判定邏輯 (`is_holding_eligible_for_ex_date` 與 `calculate_eligible_dividend_amounts`) 移動或映射至領域實體內部方法
  - [x] 將寫入 `dividend_record_detail` 與 `dividend_record_detail_more` 封裝於領域與 Repository 中
- [x] **重構 `src/app/event/taiwan_stock/ex_dividend.rs`**
  - [x] 將 `dividend::extension::stock_dividend_info` 相關的資料存取重構為透過領域或 Repository。
- [x] **重構 `src/app/event/taiwan_stock/payable_date.rs`**
  - [x] 將股利發放日的相關資料存取與提醒重構為透過領域或 Repository。

## E. 單元測試與驗證 (Testing & Validation)
- [x] **撰寫領域邏輯與 Repository 單元測試**
  - [x] 為 `Dividend` 及 `StockOwnershipDetail` 的除權息資格判定、金額計算撰寫單元測試
  - [x] 為 `PgDividendRepository` 與 `PgPortfolioRepository` 的合約行為撰寫單元測試/整合測試
- [x] **Gate 執行結果**
  - [x] `cargo check` 通過
  - [x] `cargo build` 通過
  - [x] `cargo test` 通過 (確認新增之單元測試無 regression)
