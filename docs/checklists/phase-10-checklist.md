# Phase 10: 財報與營收領域 (Financial & Revenue Domain) DDD 化 Checklist

## A. 基本資訊
- [ ] Phase 編號與名稱：Phase 10: 財報與營收領域 DDD 化
- [ ] 負責人：Antigravity
- [ ] 分支名稱：`refactor/ddd-phase-10`
- [ ] 起始 commit SHA：8c62e8f4de13a3731a5ca663d2382c730e7df6fa
- [ ] 回滾 tag 已建立：pre-stage-10

## B. 設計與定義 (Domain & Repository Definition)
- [ ] **建立 `src/domain/financial/` 目錄**
  - [ ] 建立 `src/domain/financial/mod.rs` 宣告子模組
  - [ ] 建立 `src/domain/financial/entity.rs` 定義領域實體：
    - `FinancialStatement`（季/年報、EPS）與相關商業邏輯
    - `MonthlyRevenue`（月營收）與相關商業邏輯
    - `PriceEstimate`（個股價格估值）與相關商業邏輯
  - [ ] 建立 `src/domain/financial/repository.rs` 定義 `FinancialRepository` 倉儲介面：
    - `FinancialStatement` 的讀取、存入與批次 upsert 介面
    - `MonthlyRevenue` 的讀取與存入介面
    - `PriceEstimate` 的重建與存入介面
- [ ] **更新 `src/domain/mod.rs`**
  - [ ] 匯出 `pub mod financial;`

## C. 基礎設施實現 (Infrastructure Implementation)
- [ ] **實作 `PgFinancialRepository`**
  - [ ] 於 `src/infra/database/repository/financial.rs` 實現 `FinancialRepository` 介面
  - [ ] 封裝原有 `financial_statement.rs`、`revenue.rs` 與 `estimate.rs` 的 SQL 與交易行為
- [ ] **更新 `src/infra/database/repository/mod.rs`**
  - [ ] 匯出新的 Repository 模組

## D. 應用層與事件重構 (Application Layer & Events Refactoring)
- [ ] **重構 `src/app/backfill/financial_statement/` 模組**
  - [ ] `annual.rs`：改用 `FinancialRepository` 取代 Table `upsert/batch_upsert`
  - [ ] `quarter.rs`：改用 `FinancialRepository` 進行資料存取與更新
  - [ ] `mod.rs`：改用 `FinancialRepository`
- [ ] **重構 `src/app/event/taiwan_stock/` 模組**
  - [ ] `annual_eps.rs`：重構為透過 `FinancialRepository`
  - [ ] `quarter_eps.rs`：重構為透過 `FinancialRepository`
- [ ] **重構 `src/app/backfill/revenue.rs` 與 `src/app/backfill/acl.rs`**
  - [ ] 將營收回補更新重構為透過 `FinancialRepository`
- [ ] **重構 `src/app/calculation/estimated_price.rs`**
  - [ ] 將便宜、合理、昂貴價格的估算重構為透過 `FinancialRepository`

## E. 單元測試與驗證 (Testing & Validation)
- [ ] **撰寫/更新領域邏輯與 Repository 單元測試**
  - [ ] 為 `FinancialStatement` 與 `MonthlyRevenue` 的轉換或計算邏輯撰寫單元測試
  - [ ] 為 `PgFinancialRepository` 的合約行為撰寫單元測試/整合測試
- [ ] **Gate 執行結果**
  - [ ] `cargo check` 通過
  - [ ] `cargo build` 通過
  - [ ] `cargo test` 通過 (確認新增之單元測試無 regression)
