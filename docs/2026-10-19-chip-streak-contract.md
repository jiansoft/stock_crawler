# Chip streak 與明細筆數解耦

狀態：待審查，依使用者授權提交 PR，CI 全綠後合併，未部署。配合 stock_go N01 真實服務契約驗證的最小修正，分支 `fix/chip-full-streak-contract`。

原 `/api/v1/stocks/{symbol}/chip?days=1` 先 LIMIT 一筆再計算 streak，完整連續期間因此被截成一日。現在 SQL 保留指定筆數與外資／投信各自第一個反向、零或缺值邊界，先算完整 streak，再對 daily 套用 days。HTTP 欄位與型別不變，days 仍為 1–120；streak 可超過 120。

驗證：

- 修改前在專用 PostgreSQL 16.14／Redis 7.4.11 重現 Go 25/-8、Rust days=1 為 1/-1；修正後 days=1/5/20/60、每日籌碼及股東／董監／券商區塊的 Go／Rust 契約通過。
- Rust 相關單元 5/5；原生 DB 語意測試 1/1，新增 days=1 仍回外資連買兩日的斷言。測試明確啟用後，DB 不通會失敗，不再列印跳過後算成功。
- `cargo fmt --all -- --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings` 通過。未執行本輪範圍以外的全套 Rust 測試。
- 資料庫測試使用專用 stock_round2，五個 TEST_POSTGRESQL_* 及 POSTGRESQL_* 明確指向本機 15432，未讀正式庫做驗證。

可重建流程及結果見相鄰 stock_go 的 [N01 runner](../../stock_go/scripts/integration/run.ps1) 與[第二輪說明](../../stock_go/docs/2026-10-19-round2.md)。runner 只啟動真實 HTTP router，不啟動 crawler、排程或通知；需分別審查／提交兩個專案的配套變更。
