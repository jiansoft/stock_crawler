//! 手動回補：收盤後衍生資料：未來報酬、每日相對強度、技術訊號。
//!
//! - `test_backfill_forward_returns`：
//!   逐日回填 `MANUAL_FORWARD_RETURN_FROM`～`MANUAL_FORWARD_RETURN_TO`（預設 [`MANUAL_FORWARD_RETURN_FROM`]
//!   至今天）每個交易日收盤時已滿 5／20／60／120／240 日的未來報酬；只讀資料庫，重跑安全。
//! - `test_backfill_relative_strength`：
//!   逐日寫入 `MANUAL_RELATIVE_STRENGTH_FROM`～`MANUAL_RELATIVE_STRENGTH_TO`（預設
//!   [`MANUAL_RELATIVE_STRENGTH_FROM`] 至今天）的每日相對強度（`daily_relative_strength`）；
//!   要先有未來報酬，整天替換，重跑安全。
//! - `test_backfill_technical_signals`：
//!   逐日重算全市場技術訊號（`daily_technical_signal`）。

use chrono::NaiveDate;

/// 未來報酬回填的預設起日（訊號績效統計從 2023 年起算）。
const MANUAL_FORWARD_RETURN_FROM: &str = "2023-01-01";

/// 每日相對強度回填的預設起日：未來報酬從 2023 年起算，240 日報酬約在 2024 年初才齊。
const MANUAL_RELATIVE_STRENGTH_FROM: &str = "2024-01-01";

/// 技術訊號回補的預設起日。
const MANUAL_TECHNICAL_SIGNAL_FROM: &str = "2026-09-01";

/// 技術訊號回補的預設迄日。
const MANUAL_TECHNICAL_SIGNAL_TO: &str = "2026-10-12";

/// 逐日回填未來報酬（`daily_forward_return`）。
///
/// 以每個交易日為結束日，回填往前第 5、20、60、120、240 個交易日的還原報酬；只讀資料庫既有日線、
/// 股利與公司行動，不呼叫外部網站，非交易日自動略過。起訖日可用環境變數
/// `MANUAL_FORWARD_RETURN_FROM`／`MANUAL_FORWARD_RETURN_TO` 覆寫，迄日預設今天。
/// 基準日要從 X 開始齊全，起日需早於 X 約 240 個交易日（只需 60 日以內的天期時約 60 個）。
///
/// 執行範例：
/// `MANUAL_FORWARD_RETURN_FROM=2023-01-01 cargo test app::manual_backfill::derived::test_backfill_forward_returns -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_forward_returns() {
    dotenvy::dotenv().ok();
    let today = chrono::Local::now().date_naive().to_string();
    let date = |name: &str, default: &str| {
        let raw = std::env::var(name).unwrap_or_else(|_| default.to_owned());
        NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} 格式錯誤：{raw}"))
    };
    let from = date("MANUAL_FORWARD_RETURN_FROM", MANUAL_FORWARD_RETURN_FROM);
    let to = date("MANUAL_FORWARD_RETURN_TO", &today);

    let mut total = 0;
    for day in from.iter_days().take_while(|day| *day <= to) {
        let count = crate::app::calculation::forward_return::calculate_forward_returns(day)
            .await
            .unwrap_or_else(|why| panic!("{day} 未來報酬回填失敗：{why:#}"));
        total += count;
        if count > 0 && day.format("%d").to_string() == "01" {
            println!("{day} 累計 {total} 列");
        }
    }
    println!("未來報酬回填 {from}～{to}：共 {total} 列");
}

/// 逐日寫入每日相對強度（`daily_relative_strength`）。
///
/// 每個交易日以當天的近 60／120／240 日報酬（`daily_forward_return`）對上市櫃股票（不含 ETF）計分，
/// 整天替換；報酬不齊或非交易日不寫入。只讀資料庫，不呼叫外部網站。起訖日可用環境變數
/// `MANUAL_RELATIVE_STRENGTH_FROM`／`MANUAL_RELATIVE_STRENGTH_TO` 覆寫，迄日預設今天。
///
/// 執行範例：
/// `MANUAL_RELATIVE_STRENGTH_FROM=2024-01-01 cargo test app::manual_backfill::derived::test_backfill_relative_strength -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_relative_strength() {
    dotenvy::dotenv().ok();
    let today = chrono::Local::now().date_naive().to_string();
    let date = |name: &str, default: &str| {
        let raw = std::env::var(name).unwrap_or_else(|_| default.to_owned());
        NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} 格式錯誤：{raw}"))
    };
    let from = date(
        "MANUAL_RELATIVE_STRENGTH_FROM",
        MANUAL_RELATIVE_STRENGTH_FROM,
    );
    let to = date("MANUAL_RELATIVE_STRENGTH_TO", &today);

    let (mut days, mut total) = (0, 0);
    for day in from.iter_days().take_while(|day| *day <= to) {
        let count = crate::app::calculation::relative_strength::calculate_relative_strength(day)
            .await
            .unwrap_or_else(|why| panic!("{day} 相對強度寫入失敗：{why:#}"));
        if count > 0 {
            days += 1;
            total += count;
        }
        if day.format("%d").to_string() == "01" {
            println!("{day} 累計 {days} 個交易日、{total} 列");
        }
    }
    println!("相對強度回填 {from}～{to}：{days} 個交易日、共 {total} 列");
}

/// 逐日重算全市場技術訊號（`daily_technical_signal`）。
///
/// **會覆蓋指定期間每個交易日的訊號**；只讀資料庫既有日線，不呼叫外部網站。非交易日沒有
/// 有成交的股票，不會寫入任何資料。起訖日可用環境變數 `MANUAL_TECHNICAL_SIGNAL_FROM`／
/// `MANUAL_TECHNICAL_SIGNAL_TO` 覆寫。
///
/// 執行範例：
/// `MANUAL_TECHNICAL_SIGNAL_FROM=2026-09-01 cargo test app::manual_backfill::derived::test_backfill_technical_signals -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_technical_signals() {
    dotenvy::dotenv().ok();
    let date = |name: &str, default: &str| {
        let raw = std::env::var(name).unwrap_or_else(|_| default.to_owned());
        NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} 格式錯誤：{raw}"))
    };
    let from = date("MANUAL_TECHNICAL_SIGNAL_FROM", MANUAL_TECHNICAL_SIGNAL_FROM);
    let to = date("MANUAL_TECHNICAL_SIGNAL_TO", MANUAL_TECHNICAL_SIGNAL_TO);

    for day in from.iter_days().take_while(|day| *day <= to) {
        let count = crate::app::calculation::technical_signal::calculate_technical_signals(day)
            .await
            .unwrap_or_else(|why| panic!("{day} 技術訊號計算失敗：{why:#}"));
        if count > 0 {
            println!("{day} 技術訊號 {count} 筆");
        }
    }
}
