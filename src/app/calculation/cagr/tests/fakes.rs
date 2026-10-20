//! CAGR 測試用的假資料來源、假倉儲與固定資料。

use super::*;

/// 以記憶體資料模擬 [`CagrSourceRepository`]，讓整段編排流程不需資料庫即可驗證。
#[derive(Default)]
pub(super) struct FakeSource {
    /// 全市場交易日，須由早至晚排序。
    pub(super) trading_days: Vec<NaiveDate>,
    pub(super) symbols: Vec<String>,
    /// 每檔股票的收盤價序列。
    pub(super) prices: HashMap<String, BTreeMap<NaiveDate, Decimal>>,
    pub(super) events: Vec<DividendEvent>,
    pub(super) corporate_actions: Vec<CorporateAction>,
    pub(super) anomalies: Vec<(String, NaiveDate)>,
    /// 各方法的呼叫次數，用來驗證「一次撈齊」而非 N+1 的設計。
    pub(super) calls: Mutex<HashMap<&'static str, usize>>,
}

impl FakeSource {
    pub(super) fn record(&self, name: &'static str) {
        *self
            .calls
            .lock()
            .expect("測試鎖不應中毒")
            .entry(name)
            .or_insert(0) += 1;
    }

    pub(super) fn call_count(&self, name: &str) -> usize {
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
pub(super) struct RecordingRepository {
    pub(super) saved: Mutex<Vec<StockCagr>>,
    pub(super) save_calls: Mutex<usize>,
    /// `fetch_dates_missing_period` 要回傳的基準日。
    pub(super) missing_dates: Vec<NaiveDate>,
}

impl RecordingRepository {
    pub(super) fn record_of(&self, symbol: &str, period: CagrPeriod) -> StockCagr {
        self.saved
            .lock()
            .expect("測試鎖不應中毒")
            .iter()
            .find(|row| row.stock_symbol == symbol && row.period == period)
            .cloned()
            .unwrap_or_else(|| panic!("找不到 {symbol} 的 {} 結果", period.code()))
    }

    pub(super) fn saved_len(&self) -> usize {
        self.saved.lock().expect("測試鎖不應中毒").len()
    }

    /// 已寫入資料列涵蓋的期間集合。
    pub(super) fn saved_periods(&self) -> Vec<CagrPeriod> {
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
pub(super) fn monthly_trading_days() -> Vec<NaiveDate> {
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
pub(super) fn fixture_source() -> FakeSource {
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
