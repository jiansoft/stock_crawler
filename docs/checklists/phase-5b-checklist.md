# Phase 5b: database table stock + dividend + financial grouping Checklist

## A. 基本資訊
- [x] Phase 編號與名稱：Phase 5b: database table stock + dividend + financial grouping
- [x] 負責人：Codex
- [x] 分支名稱：refactor/stage-5a-database-table-domains
- [x] 起始 commit SHA：2b50f0d
- [ ] 回滾 tag 已建立：pre-stage-5b

## B. 搬移清單 (Move List)
- [x] 搬移 `stock_exchange_market.rs` -> `stock/stock_exchange_market.rs`
- [x] 搬移 `stock_index.rs` -> `stock/stock_index.rs`
- [x] 搬移 `stock_ownership_details.rs` -> `stock/stock_ownership_details.rs`
- [x] 搬移 `stock_word.rs` -> `stock/stock_word.rs`
- [x] 搬移 `dividend_record_detail/` -> `dividend/dividend_record_detail/`
- [x] 搬移 `dividend_record_detail_more.rs` -> `dividend/dividend_record_detail_more.rs`
- [x] 建立 `src/infra/database/table/financial/`
- [x] 搬移 `financial_statement.rs` -> `financial/financial_statement.rs`
- [x] 搬移 `estimate.rs` -> `financial/estimate.rs`
- [x] 搬移 `revenue.rs` -> `financial/revenue.rs`

## C. 相容性策略
- [x] `table/mod.rs` 透過 `pub use` 保留既有 `table::<module>` 對外路徑
- [x] `stock_index` 與 `stock_word` 維持在 `stock` 內部使用，不對外 re-export
- [x] `config`、`trace`、`yield_rank`、`index` 保留於 `table/` 根層，避免硬塞不明確領域
- [x] 本階段不調整 SQL 與商業邏輯

## D. Gate 執行結果
- [x] `cargo check` 通過（stock 搬移後，2026-05-07）
- [x] `cargo check` 通過（dividend 搬移後，2026-05-07）
- [x] `cargo check` 通過（financial 搬移後，2026-05-07）
- [x] `cargo build` 通過（2026-05-07）
- [x] `cargo test -- --nocapture` 通過（2026-05-07）
- [ ] (輔助) 核心事件與計算流程 smoke test（有環境再執行）

## E. 中斷交接資訊 (Resume)
- Last Update Time: 2026-05-07 17:08:57
- Stopped At: Phase 5 Gate Completed
- Next Action: 進入 Phase 6（收尾與規範固化）。
