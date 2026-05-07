# Phase 5a: database table quote + money_flow grouping Checklist

## A. 基本資訊
- [x] Phase 編號與名稱：Phase 5a: database table quote + money_flow grouping
- [x] 負責人：Codex
- [x] 分支名稱：refactor/stage-5a-database-table-domains
- [x] 起始 commit SHA：2b50f0d
- [ ] 回滾 tag 已建立：pre-stage-5a

## B. 搬移清單 (Move List)
- [x] 建立 `src/infra/database/table/quote/`
- [x] 搬移 `daily_quote/` -> `quote/daily_quote/`
- [x] 搬移 `last_daily_quotes.rs` -> `quote/last_daily_quotes.rs`
- [x] 搬移 `quote_history_record.rs` -> `quote/quote_history_record.rs`
- [x] 搬移 `daily_stock_price_stats.rs` -> `quote/daily_stock_price_stats.rs`
- [x] 建立 `src/infra/database/table/money_flow/`
- [x] 搬移 `daily_money_history/` -> `money_flow/daily_money_history/`
- [x] 搬移 `daily_money_history_detail.rs` -> `money_flow/daily_money_history_detail.rs`
- [x] 搬移 `daily_money_history_detail_more.rs` -> `money_flow/daily_money_history_detail_more.rs`
- [x] 搬移 `daily_money_history_member.rs` -> `money_flow/daily_money_history_member.rs`

## C. 相容性策略
- [x] 新增 `quote/mod.rs` 與 `money_flow/mod.rs`
- [x] `table/mod.rs` 透過 `pub use` 保留既有 `table::<module>` 對外路徑
- [x] 本階段不調整 SQL 與商業邏輯

## D. Gate 執行結果
- [x] `cargo check` 通過（quote 搬移後，2026-05-07）
- [x] `cargo check` 通過（money_flow 搬移後，2026-05-07）
- [x] `cargo build` 通過（2026-05-07）
- [ ] (輔助) 核心事件與計算流程 smoke test（有環境再執行）

## E. 中斷交接資訊 (Resume)
- Last Update Time: 2026-05-07 17:00:09
- Stopped At: Phase 5a Gate Completed
- Next Action: Phase 5b 已接續完成，詳見 `docs/checklists/phase-5b-checklist.md`。
