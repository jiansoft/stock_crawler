//! CAGR 每日計算流程的測試。

use super::fakes::*;
use super::*;

#[tokio::test]
async fn calculate_writes_one_row_per_symbol_and_period() {
    let source = fixture_source();
    let repository = RecordingRepository::default();

    let summary = calculate(&source, &repository, None)
        .await
        .expect("計算不應失敗");

    assert_eq!(summary.date, Some(date(2026, 8, 3)));
    assert_eq!(summary.universe, 3);
    assert_eq!(summary.periods_calculated, CagrPeriod::ALL.len());
    assert_eq!(summary.periods_skipped, 0);
    // 母體中每一檔在每個期間都必須有一列，否則涵蓋率會被高估。
    assert_eq!(summary.rows_written, 27);
    assert_eq!(repository.saved_len(), 27);
    assert_eq!(summary.anomaly_symbols, 1);
}

#[tokio::test]
async fn calculate_fetches_dividend_events_only_once_for_all_periods() {
    let source = fixture_source();
    let repository = RecordingRepository::default();

    calculate(&source, &repository, None)
        .await
        .expect("計算不應失敗");

    // 八個期間共用同一次股利查詢與同一次異常偵測。
    assert_eq!(source.call_count("fetch_dividend_events_since"), 1);
    assert_eq!(source.call_count("fetch_anomaly_events"), 1);
    assert_eq!(source.call_count("fetch_closing_prices_at"), 1);
}

#[tokio::test]
async fn complete_record_carries_all_three_metrics_on_short_periods() {
    let source = fixture_source();
    let repository = RecordingRepository::default();
    calculate(&source, &repository, None)
        .await
        .expect("計算不應失敗");

    let record = repository.record_of("2330", CagrPeriod::M3);
    assert!(record.data_complete);
    assert_eq!(record.base_date, Some(date(2026, 5, 1)));
    assert_eq!(record.shortfall_days, Some(0));
    assert!(record.price.is_some(), "M3 應提供純價格口徑");
    assert!(record.total.is_some());
    assert!(record.reinvested.is_some());
    // 2019 年的疑似減資不落在三個月內，不得標記。
    assert!(!record.has_anomaly);
}

#[tokio::test]
async fn long_periods_drop_price_metric_but_keep_the_others() {
    let source = fixture_source();
    let repository = RecordingRepository::default();
    calculate(&source, &repository, None)
        .await
        .expect("計算不應失敗");

    let record = repository.record_of("2330", CagrPeriod::Y10);
    assert!(record.data_complete);
    assert!(
        record.price.is_none(),
        "長期間配股普遍，純價格口徑會系統性低估，不得提供"
    );
    assert!(record.total.is_some());
    assert!(record.reinvested.is_some());
    // 期間內的兩次除權息都要採計。
    assert_eq!(record.dividend_events, 2);
    assert!(record.has_anomaly, "2019 年的疑似減資落在十年期間內");
}

#[tokio::test]
async fn grace_period_quote_is_used_when_the_aligned_base_date_has_no_price() {
    let source = fixture_source();
    let repository = RecordingRepository::default();
    calculate(&source, &repository, None)
        .await
        .expect("計算不應失敗");

    let record = repository.record_of("1101", CagrPeriod::M3);
    assert!(record.data_complete);
    assert_eq!(record.base_date, Some(date(2026, 5, 15)));
    // 名目目標日為 2026-05-03，實際採用 05-15，落後 12 天。
    assert_eq!(record.shortfall_days, Some(12));
}

#[tokio::test]
async fn insufficient_history_is_written_as_null_row_not_zero() {
    let source = fixture_source();
    let repository = RecordingRepository::default();
    calculate(&source, &repository, None)
        .await
        .expect("計算不應失敗");

    let record = repository.record_of("1101", CagrPeriod::Y10);
    assert!(!record.data_complete);
    assert_eq!(record.base_date, None);
    assert_eq!(record.base_price, None);
    // 絕不以 0 代替，否則新股會被誤讀成「十年零報酬」。
    assert_eq!(record.years, None);
    assert_eq!(record.total, None);
    assert_eq!(record.reinvested, None);
    assert_eq!(record.dividend_events, 0);
    assert_eq!(record.first_quote_date, Some(date(2026, 5, 15)));
}

#[tokio::test]
async fn missing_end_price_keeps_base_data_but_marks_incomplete() {
    let source = fixture_source();
    let repository = RecordingRepository::default();
    calculate(&source, &repository, None)
        .await
        .expect("計算不應失敗");

    let record = repository.record_of("9999", CagrPeriod::Y1);
    assert!(!record.data_complete);
    // 期末停牌無法結算，但期初資料仍應保留以利追查。
    assert_eq!(record.base_date, Some(date(2025, 8, 1)));
    assert_eq!(record.base_price, Some(dec(20)));
    assert_eq!(record.end_price, None);
    assert_eq!(record.total, None);
}

#[tokio::test]
async fn explicit_date_is_aligned_to_the_nearest_trading_day() {
    let source = fixture_source();
    let repository = RecordingRepository::default();

    // 2026-08-05 不是交易日，應對齊到 08-03。
    let summary = calculate(&source, &repository, Some(date(2026, 8, 5)))
        .await
        .expect("計算不應失敗");

    assert_eq!(summary.date, Some(date(2026, 8, 3)));
    assert_eq!(source.call_count("fetch_latest_trading_day"), 0);
}

#[tokio::test]
async fn periods_beyond_available_history_are_skipped_entirely() {
    let mut source = fixture_source();
    // 只保留最近四個月的交易日：僅 M3 有辦法對齊期初日。
    source.trading_days.retain(|day| *day >= date(2026, 5, 1));
    let repository = RecordingRepository::default();

    let summary = calculate(&source, &repository, None)
        .await
        .expect("計算不應失敗");

    assert_eq!(summary.periods_calculated, 1);
    assert_eq!(summary.periods_skipped, CagrPeriod::ALL.len() - 1);
    // 跳過的期間不寫入任何列，避免灌進兩萬多列全 NULL 的資料。
    assert_eq!(summary.rows_written, 3);
}

#[tokio::test]
async fn no_trading_day_returns_empty_summary_without_touching_the_repository() {
    let source = FakeSource::default();
    let repository = RecordingRepository::default();

    let summary = calculate(&source, &repository, None)
        .await
        .expect("計算不應失敗");

    assert_eq!(summary, CagrCalculationSummary::default());
    assert_eq!(repository.saved_len(), 0);
    assert_eq!(source.call_count("fetch_active_symbols"), 0);
}

#[tokio::test]
async fn empty_universe_stops_before_fetching_prices() {
    let mut source = fixture_source();
    source.symbols.clear();
    let repository = RecordingRepository::default();

    let summary = calculate(&source, &repository, None)
        .await
        .expect("計算不應失敗");

    assert_eq!(summary.date, Some(date(2026, 8, 3)));
    assert_eq!(summary.universe, 0);
    assert_eq!(summary.rows_written, 0);
    assert_eq!(source.call_count("fetch_closing_prices_on"), 0);
}
