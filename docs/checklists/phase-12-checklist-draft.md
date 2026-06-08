# Phase 12: 資金流向與三大法人領域 (Money Flow Domain) DDD 化 Checklist (草案)

## A. 基本資訊
- [ ] Phase 編號與名稱：Phase 12: 資金流向與三大法人領域 DDD 化
- [ ] 負責人：Antigravity
- [ ] 分支名稱：`refactor/ddd-phase-12`
- [ ] 起始 commit SHA：(待 Phase 11 合併後填寫)
- [ ] 回滾 tag 已建立：pre-stage-12

## B. 設計與定義 (Domain & Repository Definition)
- [ ] **建立 `src/domain/money_flow/` 目錄**
  - [ ] 建立 `src/domain/money_flow/mod.rs` 宣告子模組
  - [ ] 建立 `src/domain/money_flow/entity.rs` 定義領域實體：
    - `MoneyFlow`（大盤資金流向）
    - `MoneyFlowDetail`（資金明細與法人買賣超）
    - `MoneyFlowMember`（個股/類股資金比重狀態）
  - [ ] 建立 `src/domain/money_flow/repository.rs` 定義 `MoneyFlowRepository` 倉儲介面：
    - 資金流向與細項的讀取與儲存
    - 交易式批次儲存 (Transaction-wrapped Batch Upsert)
- [ ] **更新 `src/domain/mod.rs`**
  - [ ] 匯出 `pub mod money_flow;`

## C. 基礎設施實現 (Infrastructure Implementation)
- [ ] **實作 `PgMoneyFlowRepository`**
  - [ ] 於 `src/infra/database/repository/money_flow.rs` 實現 `MoneyFlowRepository` 介面
  - [ ] 整合與封裝 `daily_money_history/`、`daily_money_history_detail.rs`、`daily_money_history_detail_more.rs` 及 `daily_money_history_member.rs` 的 SQL 表存取
  - [ ] 將原本散落在應用的複雜交易寫入移入 Repository 內部
- [ ] **更新 `src/infra/database/repository/mod.rs`**
  - [ ] 匯出新的 Repository 模組

## D. 應用層與事件重構 (Application Layer & Events Refactoring)
- [ ] **重構資金流向與三大法人回補流程**
  - [ ] 改造 `app/backfill/money_flow/` 系列任務，改為呼叫 `MoneyFlowRepository` 儲存
- [ ] **重構資金歷史計算流程**
  - [ ] 改造 `app/calculation/money_history.rs` 計算邏輯，改為使用倉儲讀寫數據
- [ ] **解耦相關事件與通知發送**
  - [ ] 檢視資金流向計算後的 Telegram 發送或日誌記錄是否可轉為領域事件

## E. 單元測試與驗證 (Testing & Validation)
- [ ] **撰寫領域邏輯與 Repository 單元測試**
  - [ ] 為資金占比、累計買賣超等領域運算撰寫單元測試
  - [ ] 為 `PgMoneyFlowRepository` 的交易儲存與多表寫入撰寫單元與整合測試
- [ ] **Gate 執行結果**
  - [ ] `cargo check` 通過
  - [ ] `cargo build` 通過
  - [ ] `cargo test` 通過 (確認無 regression)
