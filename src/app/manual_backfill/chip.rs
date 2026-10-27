//! 手動回補：籌碼：外資持股、三大法人與融資融券、集保股權分散、董監持股。
//!
//! - `test_backfill_foreign_holding_history`：
//!   依 [`MANUAL_QFII_HISTORY_START`]～[`MANUAL_QFII_HISTORY_END`] 逐日回補外資持股歷史
//!   （`qfii_history`），最後重算 `qfii_trend`；不發持股通知。
//! - `test_backfill_chip_daily`：
//!   回補每日籌碼（三大法人買賣超、融資融券餘額）到 `chip_daily`。
//! - `test_backfill_chip_streaks`：
//!   重算 `chip_daily` 的連續買賣超天數（只讀寫資料庫，不呼叫外部網站）。
//! - `test_save_holder_distributions`：
//!   立即把最新一週的集保股權分散寫入 `holder_distribution` 與 `holder_distribution_level`（排程是週六 10:40）。
//! - `test_save_insider_holdings`：
//!   立即把最新月份的董監事持股與設質寫入 `insider_holding`（排程是每月 10–28 日 20:50）。

use chrono::NaiveDate;

use crate::app::backfill::qualified_foreign_institutional_investor;

/// 回補外資持股歷史的預設起日；可用環境變數 `MANUAL_QFII_HISTORY_START` 覆蓋。
const MANUAL_QFII_HISTORY_START: &str = "2026-06-01";

/// 回補外資持股歷史的預設迄日；可用環境變數 `MANUAL_QFII_HISTORY_END` 覆蓋。
const MANUAL_QFII_HISTORY_END: &str = "2026-09-30";

/// 手動回補外資持股歷史並重算趨勢。
///
/// 逐個交易日抓證交所 `MI_QFIIS` 與櫃買 QFII API（每日間隔 3 秒，4 個月約 5 分鐘），
/// 寫入 `qfii_history` 後以最後一個交易日重算 `qfii_trend`。回補不派發持股通知。
///
/// 執行範例：
/// `MANUAL_QFII_HISTORY_START=2026-06-01 MANUAL_QFII_HISTORY_END=2026-09-30 cargo test app::manual_backfill::chip::test_backfill_foreign_holding_history -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_foreign_holding_history() {
    dotenvy::dotenv().ok();

    let parse = |name: &str, default: &str| {
        let value = std::env::var(name).unwrap_or_else(|_| default.to_string());
        NaiveDate::parse_from_str(&value, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} should be YYYY-MM-DD, got {value}"))
    };
    let start = parse("MANUAL_QFII_HISTORY_START", MANUAL_QFII_HISTORY_START);
    let end = parse("MANUAL_QFII_HISTORY_END", MANUAL_QFII_HISTORY_END);

    let summary = qualified_foreign_institutional_investor::backfill_history(start, end)
        .await
        .expect("foreign holding history backfill failed");

    println!("結束 test_backfill_foreign_holding_history {start}~{end} {summary:?}");
}

/// 回補每日籌碼（三大法人買賣超、融資融券餘額）到 `chip_daily`。
///
/// 交易日取自日報價；每天對證交所、櫃買各送兩個請求、間隔 3 秒，一年約 250 天、十幾分鐘。
/// 區間用 `MANUAL_CHIP_FROM`／`MANUAL_CHIP_TO`（`YYYY-MM-DD`）指定，未設定時沿用同名常數。
/// 只更新有差異的列，可重複執行。
///
/// 執行範例：
/// `MANUAL_CHIP_FROM=2025-10-01 cargo test app::manual_backfill::chip::test_backfill_chip_daily -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_chip_daily() {
    dotenvy::dotenv().ok();

    let parse = |name: &str, default: &str| {
        let value = std::env::var(name).unwrap_or_else(|_| default.to_string());
        NaiveDate::parse_from_str(&value, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} should be YYYY-MM-DD, got {value}"))
    };
    let from = parse("MANUAL_CHIP_FROM", MANUAL_CHIP_FROM);
    let to = parse("MANUAL_CHIP_TO", MANUAL_CHIP_TO);

    println!("開始 test_backfill_chip_daily from={from} to={to}");
    let written = crate::app::backfill::chip::backfill_daily(from, to)
        .await
        .expect("manual chip backfill failed");
    println!("結束 test_backfill_chip_daily from={from} to={to} rows_written={written}");
}

/// 逐年重算 `chip_daily` 的外資、投信、自營商與合計連續買賣超天數。
///
/// 只讀寫資料庫、不呼叫外部網站；只更新有變的列，可重複執行。起日用 `MANUAL_CHIP_STREAK_FROM`
/// 指定（預設 [`MANUAL_CHIP_STREAK_FROM`]，即籌碼資料最早一天），一路重算到今天。
///
/// 執行範例：
/// `MANUAL_CHIP_STREAK_FROM=2019-01-02 cargo test app::manual_backfill::chip::test_backfill_chip_streaks -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_chip_streaks() {
    dotenvy::dotenv().ok();

    let raw = std::env::var("MANUAL_CHIP_STREAK_FROM")
        .unwrap_or_else(|_| MANUAL_CHIP_STREAK_FROM.to_string());
    let from = NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
        .unwrap_or_else(|_| panic!("MANUAL_CHIP_STREAK_FROM should be YYYY-MM-DD, got {raw}"));
    let to = chrono::Local::now().date_naive();

    let updated = crate::app::backfill::chip::refresh_streaks_since(
        &crate::infra::database::repository::chip::PgChipRepository::new(),
        from,
        to,
    )
    .await
    .expect("chip streak refresh failed");
    println!("結束 test_backfill_chip_streaks {from}～{to} rows_updated={updated}");
}

/// 重算連續買賣超天數的預設起日：`chip_daily` 最早一天。
const MANUAL_CHIP_STREAK_FROM: &str = "2019-01-02";

/// 每日籌碼回補的預設區間（可用環境變數覆寫）。
const MANUAL_CHIP_FROM: &str = "2025-10-01";

const MANUAL_CHIP_TO: &str = "2026-10-06";

/// 立即把最新一週的集保股權分散寫入 `holder_distribution`（週摘要）與
/// `holder_distribution_level`（全部持股分級），排程是週六 10:40。
///
/// 執行範例：
/// `cargo test app::manual_backfill::chip::test_save_holder_distributions -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_save_holder_distributions() {
    dotenvy::dotenv().ok();
    crate::app::backfill::chip::execute_holder_distributions()
        .await
        .expect("save holder distributions failed");
}

/// 立即把最新月份的董監事持股與設質寫入 `insider_holding`（排程是每月 10–28 日 20:50）。
///
/// 執行範例：
/// `cargo test app::manual_backfill::chip::test_save_insider_holdings -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_save_insider_holdings() {
    dotenvy::dotenv().ok();
    crate::app::backfill::chip::execute_insider_holdings()
        .await
        .expect("save insider holdings failed");
}
