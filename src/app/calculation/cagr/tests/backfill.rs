//! CAGR 指定期間回補與純函式的測試。

use super::fakes::*;
use super::*;

#[tokio::test]
async fn backfill_period_only_calculates_the_requested_period() {
    let source = fixture_source();
    let repository = RecordingRepository {
        missing_dates: vec![date(2026, 8, 3)],
        ..Default::default()
    };

    let summary = backfill_period_with(&source, &repository, CagrPeriod::Y7)
        .await
        .expect("回填不應失敗");

    assert_eq!(summary.dates_pending, 1);
    assert_eq!(summary.dates_processed, 1);
    assert_eq!(summary.dates_skipped, 0);
    // 母體三檔 × 只有 Y7 一個期間。
    assert_eq!(summary.rows_written, 3);
    assert_eq!(
        repository.saved_periods(),
        vec![CagrPeriod::Y7],
        "回填不得順便重算其他期間"
    );
}

#[tokio::test]
async fn backfill_period_processes_every_pending_date() {
    let source = fixture_source();
    let repository = RecordingRepository {
        missing_dates: vec![date(2026, 7, 1), date(2026, 8, 3)],
        ..Default::default()
    };

    let summary = backfill_period_with(&source, &repository, CagrPeriod::Y1)
        .await
        .expect("回填不應失敗");

    assert_eq!(summary.dates_pending, 2);
    assert_eq!(summary.dates_processed, 2);
    assert_eq!(summary.rows_written, 6);
}

#[tokio::test]
async fn backfill_period_counts_dates_without_enough_history_as_skipped() {
    let mut source = fixture_source();
    // 只留最近四個月的交易日：Y10 的期初日無從對齊。
    source.trading_days.retain(|day| *day >= date(2026, 5, 1));
    let repository = RecordingRepository {
        missing_dates: vec![date(2026, 8, 3)],
        ..Default::default()
    };

    let summary = backfill_period_with(&source, &repository, CagrPeriod::Y10)
        .await
        .expect("回填不應失敗");

    assert_eq!(summary.dates_skipped, 1);
    assert_eq!(summary.dates_processed, 0);
    assert_eq!(summary.rows_written, 0);
    assert_eq!(repository.saved_len(), 0);
}

#[tokio::test]
async fn backfill_period_is_a_no_op_when_nothing_is_missing() {
    let source = fixture_source();
    let repository = RecordingRepository::default();

    let summary = backfill_period_with(&source, &repository, CagrPeriod::Y7)
        .await
        .expect("回填不應失敗");

    assert_eq!(summary, CagrPeriodBackfillSummary::default());
    // 沒有待回填的日期時，連交易日都不該去查。
    assert_eq!(source.call_count("fetch_latest_trading_day"), 0);
    assert_eq!(source.call_count("fetch_active_symbols"), 0);
}

#[test]
fn collect_reinvest_pairs_keeps_only_in_range_cash_dividends() {
    let from = date(2024, 1, 1);
    let to = date(2024, 12, 31);
    let event = |day: NaiveDate, cash: Decimal| DividendEvent {
        stock_symbol: "2330".to_string(),
        ex_dividend_date_cash: Some(day),
        ex_dividend_date_stock: None,
        cash_dividend: cash,
        stock_dividend: Decimal::ZERO,
    };

    let mut events_by_symbol = HashMap::new();
    events_by_symbol.insert(
        "2330".to_string(),
        vec![
            event(date(2024, 6, 20), dec(5)),
            // 同一天重複出現只需查一次價。
            event(date(2024, 6, 20), dec(5)),
            // 期初日當天不生效（區間為左開右閉）。
            event(from, dec(5)),
            // 期末之後不生效。
            event(date(2025, 1, 2), dec(5)),
            // 沒有現金股利的事件不需查價。
            event(date(2024, 8, 15), Decimal::ZERO),
            // 只有除權日的事件不需查價。
            DividendEvent {
                stock_symbol: "2330".to_string(),
                ex_dividend_date_cash: None,
                ex_dividend_date_stock: Some(date(2024, 7, 1)),
                cash_dividend: Decimal::ZERO,
                stock_dividend: dec(1),
            },
        ],
    );

    let pairs = collect_reinvest_pairs(&events_by_symbol, from, to);

    assert_eq!(pairs, vec![("2330".to_string(), date(2024, 6, 20))]);
}

#[test]
fn group_helpers_index_by_symbol() {
    let events = vec![
        DividendEvent {
            stock_symbol: "2330".to_string(),
            ex_dividend_date_cash: Some(date(2024, 6, 20)),
            ex_dividend_date_stock: None,
            cash_dividend: dec(5),
            stock_dividend: Decimal::ZERO,
        },
        DividendEvent {
            stock_symbol: "2330".to_string(),
            ex_dividend_date_cash: Some(date(2023, 6, 20)),
            ex_dividend_date_stock: None,
            cash_dividend: dec(3),
            stock_dividend: Decimal::ZERO,
        },
        DividendEvent {
            stock_symbol: "1101".to_string(),
            ex_dividend_date_cash: Some(date(2024, 7, 10)),
            ex_dividend_date_stock: None,
            cash_dividend: dec(1),
            stock_dividend: Decimal::ZERO,
        },
    ];

    let grouped = group_events_by_symbol(events);
    assert_eq!(grouped["2330"].len(), 2);
    assert_eq!(grouped["1101"].len(), 1);

    let prices = group_prices_by_symbol(vec![
        ("2330".to_string(), date(2024, 6, 20), dec(120)),
        ("2330".to_string(), date(2023, 6, 20), dec(100)),
        ("1101".to_string(), date(2024, 7, 10), dec(30)),
    ]);
    assert_eq!(prices["2330"].len(), 2);
    assert_eq!(prices["2330"][&date(2024, 6, 20)], dec(120));
    assert_eq!(prices["1101"].len(), 1);
}
