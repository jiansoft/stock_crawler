//! CAGR 計算流程的單元測試（以假資料來源 `FakeSource` 取代資料庫）。

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;

use super::*;
use crate::domain::performance::StockCagr;
use crate::domain::performance::entity::{CagrCoverage, CagrMetric};
use crate::domain::performance::query::{CagrRankingPage, CagrRankingQuery};

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
}

fn dec(value: i64) -> Decimal {
    Decimal::from(value)
}

/// 以記憶體資料模擬 [`CagrSourceRepository`]，讓整段編排流程不需資料庫即可驗證。
#[derive(Default)]
struct FakeSource {
    /// 全市場交易日，須由早至晚排序。
    trading_days: Vec<NaiveDate>,
    symbols: Vec<String>,
    /// 每檔股票的收盤價序列。
    prices: HashMap<String, BTreeMap<NaiveDate, Decimal>>,
    events: Vec<DividendEvent>,
    corporate_actions: Vec<CorporateAction>,
    anomalies: Vec<(String, NaiveDate)>,
    /// 各方法的呼叫次數，用來驗證「一次撈齊」而非 N+1 的設計。
    calls: Mutex<HashMap<&'static str, usize>>,
}

impl FakeSource {
    fn record(&self, name: &'static str) {
        *self
            .calls
            .lock()
            .expect("測試鎖不應中毒")
            .entry(name)
            .or_insert(0) += 1;
    }

    fn call_count(&self, name: &str) -> usize {
        self.calls
            .lock()
            .expect("測試鎖不應中毒")
            .get(name)
            .copied()
            .unwrap_or(0)
    }
}

#[async_trait]
impl CagrSourceRepository for FakeSource {
    async fn fetch_trading_day_on_or_before(&self, date: NaiveDate) -> Result<Option<NaiveDate>> {
        self.record("fetch_trading_day_on_or_before");
        Ok(self
            .trading_days
            .iter()
            .filter(|day| **day <= date)
            .max()
            .copied())
    }

    async fn fetch_latest_trading_day(&self) -> Result<Option<NaiveDate>> {
        self.record("fetch_latest_trading_day");
        Ok(self.trading_days.iter().max().copied())
    }

    async fn fetch_active_symbols(&self) -> Result<Vec<String>> {
        self.record("fetch_active_symbols");
        Ok(self.symbols.clone())
    }

    async fn fetch_first_quote_dates(&self) -> Result<Vec<(String, NaiveDate)>> {
        self.record("fetch_first_quote_dates");
        Ok(self
            .prices
            .iter()
            .filter_map(|(symbol, series)| {
                series.keys().next().map(|first| (symbol.clone(), *first))
            })
            .collect())
    }

    async fn fetch_closing_prices_on(&self, date: NaiveDate) -> Result<Vec<(String, Decimal)>> {
        self.record("fetch_closing_prices_on");
        Ok(self
            .prices
            .iter()
            .filter_map(|(symbol, series)| series.get(&date).map(|price| (symbol.clone(), *price)))
            .collect())
    }

    async fn fetch_first_quote_within(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<(String, NaiveDate, Decimal)>> {
        self.record("fetch_first_quote_within");
        Ok(self
            .prices
            .iter()
            .filter_map(|(symbol, series)| {
                series
                    .range(from..=to)
                    .next()
                    .map(|(quote_date, price)| (symbol.clone(), *quote_date, *price))
            })
            .collect())
    }

    async fn fetch_dividend_events_since(&self, since: NaiveDate) -> Result<Vec<DividendEvent>> {
        self.record("fetch_dividend_events_since");
        Ok(self
            .events
            .iter()
            .filter(|event| event.sort_key().is_some_and(|day| day >= since))
            .cloned()
            .collect())
    }

    async fn fetch_corporate_actions_since(
        &self,
        since: NaiveDate,
    ) -> Result<Vec<CorporateAction>> {
        self.record("fetch_corporate_actions_since");
        Ok(self
            .corporate_actions
            .iter()
            .filter(|action| action.effective_date > since)
            .cloned()
            .collect())
    }

    async fn fetch_closing_prices_at(
        &self,
        pairs: &[(String, NaiveDate)],
    ) -> Result<Vec<(String, NaiveDate, Decimal)>> {
        self.record("fetch_closing_prices_at");
        Ok(pairs
            .iter()
            .filter_map(|(symbol, day)| {
                self.prices
                    .get(symbol)
                    .and_then(|series| series.get(day))
                    .map(|price| (symbol.clone(), *day, *price))
            })
            .collect())
    }

    async fn fetch_anomaly_events(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<(String, NaiveDate)>> {
        self.record("fetch_anomaly_events");
        Ok(self
            .anomalies
            .iter()
            .filter(|(_, day)| *day > from && *day <= to)
            .cloned()
            .collect())
    }
}

/// 只記錄寫入內容的 [`CagrRepository`]；讀取類方法在本測試不會被呼叫。
#[derive(Default)]
struct RecordingRepository {
    saved: Mutex<Vec<StockCagr>>,
    save_calls: Mutex<usize>,
    /// `fetch_dates_missing_period` 要回傳的基準日。
    missing_dates: Vec<NaiveDate>,
}

impl RecordingRepository {
    fn record_of(&self, symbol: &str, period: CagrPeriod) -> StockCagr {
        self.saved
            .lock()
            .expect("測試鎖不應中毒")
            .iter()
            .find(|row| row.stock_symbol == symbol && row.period == period)
            .cloned()
            .unwrap_or_else(|| panic!("找不到 {symbol} 的 {} 結果", period.code()))
    }

    fn saved_len(&self) -> usize {
        self.saved.lock().expect("測試鎖不應中毒").len()
    }

    /// 已寫入資料列涵蓋的期間集合。
    fn saved_periods(&self) -> Vec<CagrPeriod> {
        let mut periods: Vec<CagrPeriod> = self
            .saved
            .lock()
            .expect("測試鎖不應中毒")
            .iter()
            .map(|row| row.period)
            .collect();
        periods.sort_unstable();
        periods.dedup();
        periods
    }
}

#[async_trait]
impl CagrRepository for RecordingRepository {
    async fn save_batch(&self, records: &[StockCagr]) -> Result<u64> {
        *self.save_calls.lock().expect("測試鎖不應中毒") += 1;
        self.saved
            .lock()
            .expect("測試鎖不應中毒")
            .extend_from_slice(records);
        Ok(records.len() as u64)
    }

    async fn fetch_ranking(
        &self,
        _date: NaiveDate,
        _period: CagrPeriod,
        _metric: CagrMetric,
        _limit: i64,
        _offset: i64,
    ) -> Result<Vec<StockCagr>> {
        unimplemented!("計算流程不會讀取排行榜")
    }

    async fn fetch_ranking_page(&self, _query: &CagrRankingQuery) -> Result<CagrRankingPage> {
        unimplemented!("計算流程不會讀取排行榜")
    }

    async fn fetch_by_symbol(
        &self,
        _date: NaiveDate,
        _stock_symbol: &str,
    ) -> Result<Vec<StockCagr>> {
        unimplemented!("計算流程不會讀取個股結果")
    }

    async fn fetch_coverage(
        &self,
        _date: NaiveDate,
        _period: CagrPeriod,
        _metric: CagrMetric,
    ) -> Result<CagrCoverage> {
        unimplemented!("計算流程不會讀取涵蓋統計")
    }

    async fn fetch_latest_date(&self) -> Result<Option<NaiveDate>> {
        unimplemented!("計算流程不會讀取最新基準日")
    }

    async fn fetch_dates_missing_period(&self, _period: CagrPeriod) -> Result<Vec<NaiveDate>> {
        Ok(self.missing_dates.clone())
    }

    async fn delete_before(&self, _date: NaiveDate) -> Result<u64> {
        unimplemented!("計算流程不做保留期清理")
    }
}

/// 建立 2015-01 起每月 1 日的交易日，最後補上期末日 2026-08-03。
fn monthly_trading_days() -> Vec<NaiveDate> {
    let mut days = Vec::new();
    for year in 2015..=2026 {
        for month in 1..=12 {
            let day = date(year, month, 1);
            if day <= date(2026, 8, 1) {
                days.push(day);
            }
        }
    }
    days.push(date(2026, 5, 15));
    days.push(date(2026, 8, 3));
    days.sort_unstable();
    days
}

/// 三檔股票，各自對應一種待驗證情境：
///
/// - `2330`：全期間都有報價，價格逐月上升，且有除權息與一次疑似減資
/// - `1101`：期初日當天無報價，但落在 30 天寬限期內（走寬限分支）
/// - `9999`：有期初價但期末日停牌（走「期末無報價」分支）
fn fixture_source() -> FakeSource {
    let trading_days = monthly_trading_days();

    let mut prices: HashMap<String, BTreeMap<NaiveDate, Decimal>> = HashMap::new();
    let series_2330: BTreeMap<NaiveDate, Decimal> = trading_days
        .iter()
        .enumerate()
        .map(|(index, day)| (*day, dec(50 + index as i64)))
        .collect();
    prices.insert("2330".to_string(), series_2330);

    // 除息日不是月初，另外補進價格序列供再投入口徑查價。
    prices
        .get_mut("2330")
        .expect("2330 序列應存在")
        .insert(date(2024, 6, 20), dec(120));

    prices.insert(
        "1101".to_string(),
        trading_days
            .iter()
            .filter(|day| **day >= date(2026, 5, 15))
            .map(|day| (*day, dec(30)))
            .collect(),
    );

    prices.insert(
        "9999".to_string(),
        trading_days
            .iter()
            .filter(|day| **day < date(2026, 8, 3))
            .map(|day| (*day, dec(20)))
            .collect(),
    );

    FakeSource {
        trading_days,
        symbols: vec!["2330".to_string(), "1101".to_string(), "9999".to_string()],
        prices,
        events: vec![
            DividendEvent {
                stock_symbol: "2330".to_string(),
                ex_dividend_date_cash: Some(date(2024, 6, 20)),
                ex_dividend_date_stock: None,
                cash_dividend: dec(5),
                stock_dividend: Decimal::ZERO,
            },
            DividendEvent {
                stock_symbol: "2330".to_string(),
                ex_dividend_date_stock: Some(date(2019, 7, 1)),
                ex_dividend_date_cash: None,
                cash_dividend: Decimal::ZERO,
                stock_dividend: dec(1),
            },
        ],
        // 2020-05-04 的 1:2 分割：模擬時應調整股數，且不得再標記為異常。
        corporate_actions: vec![CorporateAction {
            stock_symbol: "2330".to_string(),
            effective_date: date(2020, 5, 4),
            action_type: crate::domain::performance::CorporateActionType::Split,
            share_ratio: dec(2),
            note: "1:2 分割".to_string(),
        }],
        anomalies: vec![("2330".to_string(), date(2019, 3, 1))],
        calls: Mutex::default(),
    }
}

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
