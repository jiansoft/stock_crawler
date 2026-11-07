//! 上櫃減資待辦清單的測試：計數、文字輸出，與連資料庫的掃描（只留上櫃、附單日變動率）。

use super::*;
use rust_decimal_macros::dec;

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
}

fn sample_report() -> PendingReport {
    PendingReport {
        from: date(2015, 1, 1),
        to: date(2026, 9, 17),
        total_events: 120,
        skipped_non_otc: 100,
        symbols: vec![
            PendingSymbol {
                stock_symbol: "3710".to_string(),
                name: "連展投控".to_string(),
                events: vec![
                    PendingEvent {
                        event_date: date(2020, 5, 4),
                        change_percent: dec!(62.1234),
                    },
                    PendingEvent {
                        event_date: date(2023, 8, 1),
                        change_percent: dec!(35.5),
                    },
                ],
            },
            PendingSymbol {
                stock_symbol: "8277".to_string(),
                name: "商丞".to_string(),
                events: vec![PendingEvent {
                    event_date: date(2019, 3, 1),
                    change_percent: dec!(-40.0),
                }],
            },
        ],
    }
}

#[test]
fn counts_symbols_and_events() {
    let report = sample_report();

    assert_eq!(report.symbol_count(), 2);
    assert_eq!(report.event_count(), 3);
}

#[test]
fn empty_report_counts_zero() {
    let report = PendingReport::default();

    assert_eq!(report.symbol_count(), 0);
    assert_eq!(report.event_count(), 0);
}

/// 報告要能一眼看出規模與每檔的事件日期，變動率取到小數兩位。
#[test]
fn format_report_lists_every_event() {
    let text = format_report(&sample_report());

    assert!(text.contains("原始異常事件 120 筆"));
    assert!(text.contains("非上櫃濾除 100 筆"));
    assert!(text.contains("剩 2 檔 / 3 筆事件"));
    assert!(text.contains("3710 連展投控：2020-05-04 (62.12%)、2023-08-01 (35.50%)"));
    assert!(
        text.contains("8277 商丞：2019-03-01 (-40.00%)"),
        "小數位要補零對齊，實際輸出：{text}"
    );
}

/// 沒有待辦時只輸出摘要行，不應有額外空行造成誤讀。
#[test]
fn format_report_handles_empty_symbols() {
    let report = PendingReport {
        from: date(2015, 1, 1),
        to: date(2026, 9, 17),
        total_events: 0,
        skipped_non_otc: 0,
        symbols: Vec::new(),
    };

    let text = format_report(&report);

    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("剩 0 檔 / 0 筆事件"));
}

/// 上櫃、上市與股票主檔沒有的假代號各一檔，1989-01 的第 7 個交易日都從 10 元跳到 20 元。
const OTC: &str = "79979CRA";
const LISTED: &str = "79979CRB";
const UNKNOWN: &str = "79979CRC";

async fn cleanup_scan_fixtures() {
    let pool = crate::infra::database::get_connection();
    let symbols = vec![OTC.to_string(), LISTED.to_string(), UNKNOWN.to_string()];
    for sql in [
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        sqlx::query(sql)
            .bind(&symbols)
            .execute(pool)
            .await
            .expect("清理測試資料");
    }
}

/// 連資料庫：異常跳動只留上櫃的那檔（上市與主檔查不到的計入非上櫃濾除），並附當天的單日變動率。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn scan_keeps_only_otc_jumps_with_change_percent() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 scan_keeps_only_otc_jumps_with_change_percent：無資料庫連接");
        return;
    }
    cleanup_scan_fixtures().await;
    for (symbol, market_id) in [(OTC, Some(4)), (LISTED, Some(2)), (UNKNOWN, None)] {
        if let Some(market_id) = market_id {
            sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing")
               VALUES ($1, $1, $1, $2, false)"#,
        )
        .bind(symbol)
        .bind(market_id)
        .execute(pool)
        .await
        .expect("寫入測試股票");
        }
        for d in 2..=8 {
            let close = if d == 8 { dec!(20) } else { dec!(10) };
            sqlx::query(
                r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "TradingVolume")
                   VALUES ($1, $2, $3, 1000)"#,
            )
            .bind(date(1989, 1, d))
            .bind(symbol)
            .bind(close)
            .execute(pool)
            .await
            .expect("寫入測試日線");
        }
    }
    SHARE.load().await;

    let report = scan(date(1989, 1, 1), date(1989, 1, 31)).await;

    cleanup_scan_fixtures().await;
    SHARE.load().await;
    let report = report.expect("掃描成功");
    assert_eq!(report.total_events, 3, "{report:?}");
    assert_eq!(report.skipped_non_otc, 2, "上市與主檔查不到的濾除");
    assert_eq!(report.symbol_count(), 1);
    let symbol = &report.symbols[0];
    assert_eq!(symbol.stock_symbol, OTC);
    assert_eq!(symbol.events.len(), 1);
    assert_eq!(symbol.events[0].event_date, date(1989, 1, 8));
    assert_eq!(symbol.events[0].change_percent, dec!(100));
}
