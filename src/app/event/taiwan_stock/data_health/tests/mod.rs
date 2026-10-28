//! 資料健康週報的測試：落後天數判斷、衍生表與未來報酬缺漏、訊息格式。

use super::*;

mod derived;
mod etf;
mod lagging;
mod periodic;
mod trading;

/// 對 `.env` 的資料庫量一次並印出週報，不發送（唯讀）。
///
/// `cargo test app::event::taiwan_stock::data_health::tests::print_report -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn print_report() {
    dotenvy::dotenv().ok();
    let message = build_report(Local::now().date_naive())
        .await
        .expect("量測應成功");
    println!("{message}");
}
use crate::domain::{
    health::{DerivedTableLatest, EtfNavDayCount},
    quote::entity::MarketTradedCount,
};

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("日期應合法")
}

fn count(date: NaiveDate, market_id: i32, traded: i64) -> MarketTradedCount {
    MarketTradedCount {
        date,
        market_id,
        traded,
    }
}

/// 2026-10-05（一）～10-09（五）都正常、衍生表都到 10-09 的量測值；
/// 基準期含 10-02（五），未來報酬的 5 日報酬應到這天。
fn healthy() -> DataHealthSnapshot {
    let days = [day(5), day(6), day(7), day(8), day(9)];
    DataHealthSnapshot {
        market_counts: std::iter::once(day(2))
            .chain(days)
            .flat_map(|date| [count(date, 2, 1_000), count(date, 4, 880)])
            .collect(),
        quoted_dates: days.to_vec(),
        derived_tables: [
            ("estimate", day(9)),
            ("yield_rank", day(9)),
            ("daily_stock_price_stats", day(9)),
            ("last_daily_quotes", day(9)),
            ("stock_cagr", day(8)),
            ("qfii_history", day(8)),
            ("daily_technical_signal", day(9)),
            ("daily_relative_strength", day(9)),
            ("chip_daily", day(8)),
            ("broker_flow", day(8)),
            ("daily_market_valuation", day(9)),
            ("etf_performance", day(9)),
            ("etf_snapshot", day(9)),
            ("daily_forward_return", day(2)),
        ]
        .into_iter()
        .map(|(table, latest)| DerivedTableLatest {
            table,
            latest: Some(latest),
        })
        .collect(),
        latest_revenue_month: Some(202609),
        latest_holder_week: Some(day(2)),
        latest_holder_level_week: Some(day(2)),
        holder_level_rows: 69_700,
        latest_insider_month: NaiveDate::from_ymd_opt(2026, 8, 1),
        // 最新交易日 10-09 的正式淨值下一個營業日才回填。
        etf_nav_counts: std::iter::once(day(2))
            .chain(days)
            .map(|date| etf_nav(date, 360, if date == day(9) { 360 } else { 1 }))
            .collect(),
        ..Default::default()
    }
}

fn etf_nav(date: NaiveDate, rows: i64, missing_nav: i64) -> EtfNavDayCount {
    EtfNavDayCount {
        date,
        rows,
        missing_nav,
    }
}

fn problems_of<'a>(checks: &'a [Check], name: &str) -> &'a [String] {
    &checks
        .iter()
        .find(|check| check.name == name)
        .expect("應有此項目")
        .problems
}

/// 全部正常時每一項都沒有問題，週報只說「全部正常」。
#[test]
fn a_healthy_week_reports_no_problems() {
    let checks = evaluate(&healthy(), day(4), day(10), Some(&[]));
    assert!(
        checks.iter().all(|check| check.problems.is_empty()),
        "{checks:?}"
    );

    let message = report(day(4), day(10), &checks);
    assert!(
        message.starts_with("資料健康週報 2026\\-10\\-04～2026\\-10\\-10\n全部 12 項正常"),
        "{message}"
    );
    assert!(!message.contains("⚠️"));
    // 正常時也附上集保級距的數字，週六可以直接確認這週有寫入。
    assert!(
        message.contains("✅ 週／月資料（集保級距 2026\\-10\\-02 69700 列、400 張空值 0 檔）"),
        "{message}"
    );
}

/// 計數類項目大於 0 才列出，週報標出需要處理的項目數。
#[test]
fn counted_problems_are_listed_in_the_report() {
    let mut snapshot = healthy();
    snapshot.missing_moving_averages = 12;
    snapshot.stale_dividend_placeholders = 3;
    snapshot.dividend_total_mismatches = 1;
    snapshot.chip_streak_missing = 1_890;

    let checks = evaluate(&snapshot, day(4), day(10), Some(&[]));
    let message = report(day(4), day(10), &checks);

    assert!(message.contains("3 項需要處理"), "{message}");
    assert!(
        message.contains("⚠️ 籌碼連續天數\n  • 1890 列連續買賣超天數未計算"),
        "{message}"
    );
    assert!(
        message.contains("⚠️ 均線\n  • 12 列 5 日均線是 0"),
        "{message}"
    );
    assert!(
        message.contains(
            "⚠️ 股利\n  • 近五年有 3 列發放年度已過，除權息日仍是「尚未公布」\n  • 1 組年度合計與各期明細加總不符"
        ),
        "{message}"
    );
    assert!(message.contains("✅ 漲跌幅"), "{message}");
    // MarkdownV2 保留字元都要跳脫，否則 Telegram 會整則拒收。
    assert!(
        message.contains("test\\_recalculate\\_moving\\_averages"),
        "{message}"
    );
}
