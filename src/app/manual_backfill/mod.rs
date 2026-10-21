//! 手動資料回補測試入口。
//!
//! 這個模組集中放置平常不應自動執行、但缺資料時可用 `cargo test ... -- --ignored`
//! 直接觸發的回補測試。每個測試都依賴本機 `.env`、資料庫與外部資料來源，
//! 因此一律標記為 `#[ignore]`。
//!
//! 依資料類別分成子模組，每個入口的說明見各子模組：
//!
//! - [`quote`]：日報價、收盤匯總、均線、大盤指數、官方本益比與除權息漲跌
//! - [`dividend`]：股利、已領股利與除權息核對
//! - [`cagr`]：年化報酬率（CAGR）
//! - [`capital_reduction`]：減資事件
//! - [`financial`]：三大財務報表與財務體質
//! - [`chip`]：籌碼：外資持股、三大法人與融資融券、集保股權分散、董監持股
//! - [`derived`]：收盤後衍生資料：未來報酬、每日相對強度、技術訊號
//! - [`etf`]：ETF 專區：基本資料與持股、淨值與績效、快照
//!
//! 測試名稱不重複，執行時可只用名稱篩選，例如
//! `cargo test --locked test_backfill_chip_daily -- --ignored --nocapture`。

mod cagr;
mod capital_reduction;
mod chip;
mod derived;
mod dividend;
mod etf;
mod financial;
mod quote;
