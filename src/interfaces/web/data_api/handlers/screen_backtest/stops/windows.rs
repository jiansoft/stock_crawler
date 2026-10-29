//! 停損與日頻報酬用的日線：持有期間的讀取視窗、還原權值、開盤價比與跨期共用的日線快取。

use super::*;

/// 各股在指定交易日的開盤價／收盤價比，鍵為 `(代號, 日期)`；次日開盤成交的日頻路徑用。
pub(in super::super) type OpenRatios = HashMap<(String, NaiveDate), f64>;

/// 換股日之後的第一個交易日（`calendar` 為 0050 或全部日線的交易日）。
pub(in super::super) fn entry_date(
    calendar: &std::collections::BTreeSet<NaiveDate>,
    rebalance: NaiveDate,
) -> Option<NaiveDate> {
    calendar
        .range((
            std::ops::Bound::Excluded(rebalance),
            std::ops::Bound::Unbounded,
        ))
        .next()
        .copied()
}

/// 次日開盤成交時，各期持股與 0050 在進場日（次一交易日）與出場日的開盤價／收盤價比。
pub(in super::super) async fn load_open_ratios(
    rows: &[PeriodRow],
    bars: &HashMap<String, Arc<Vec<PriceBar>>>,
) -> Result<OpenRatios, sqlx::Error> {
    let calendar: std::collections::BTreeSet<NaiveDate> = bars
        .get(super::super::ETF_BENCHMARK)
        .map(|series| series.iter().map(|bar| bar.date).collect())
        .unwrap_or_default();
    let mut points: HashSet<(String, NaiveDate)> = HashSet::new();
    for row in rows.iter().filter(|row| !row.pending) {
        let (Some(exit), Some(entry)) = (
            row.exit_date,
            row.entry_date.or_else(|| entry_date(&calendar, row.date)),
        ) else {
            continue;
        };
        for symbol in row
            .symbols
            .iter()
            .map(String::as_str)
            .chain([super::super::ETF_BENCHMARK])
        {
            points.insert((symbol.to_owned(), entry));
            points.insert((symbol.to_owned(), exit));
        }
    }
    let points: Vec<(String, NaiveDate)> = points.into_iter().collect();
    PgQuoteRepository::new().fetch_open_ratios(&points).await
}

/// 持有期間往前多讀的日曆天：換股日當天停牌時，沿用之前最近的收盤當買進價。
pub(super) const WINDOW_LOOKBACK_DAYS: i64 = 30;

/// 只算日頻路徑（沒有停損）時的日線：各持股只讀「換股日前 [`WINDOW_LOOKBACK_DAYS`] 天～出場日」，
/// [`super::super::ETF_BENCHMARK`] 讀整段當交易日曆；同一檔重疊或相鄰的期間先合併成一段。
///
/// 每段各自還原：日頻路徑只比較同一期內的收盤，段內的相對報酬與整段還原相同。
pub(in super::super) async fn load_holding_windows(
    rows: &[PeriodRow],
) -> Result<HashMap<String, Arc<Vec<PriceBar>>>, sqlx::Error> {
    let windows = holding_windows(rows);
    if windows.is_empty() {
        return Ok(HashMap::new());
    }
    let repository = PgQuoteRepository::new();
    let raw = repository.fetch_bar_windows(&windows).await?;
    let symbols: Vec<String> = raw.keys().cloned().collect();
    let adjustments = repository.price_adjustments(&symbols).await?;
    Ok(raw
        .into_iter()
        .map(|(symbol, series)| {
            let events = adjustments.get(&symbol).map_or(&[][..], Vec::as_slice);
            let adjusted = adjust_windows(&series, &windows, &symbol, events);
            (symbol, Arc::new(adjusted))
        })
        .collect())
}

/// 已滿持有天數的各期持股要讀的日線期間 `(代號, 起日, 迄日)`，同一檔合併成不重疊的段、依代號排序。
pub(in super::super) fn holding_windows(rows: &[PeriodRow]) -> Vec<(String, NaiveDate, NaiveDate)> {
    let lookback = chrono::Duration::days(WINDOW_LOOKBACK_DAYS);
    let mut spans: HashMap<&str, Vec<(NaiveDate, NaiveDate)>> = HashMap::new();
    let mut calendar: Option<(NaiveDate, NaiveDate)> = None;
    for row in rows.iter().filter(|row| !row.pending) {
        let Some(exit) = row.exit_date else {
            continue;
        };
        let start = row.date - lookback;
        calendar = Some(calendar.map_or((start, exit), |(s, e)| (s.min(start), e.max(exit))));
        for symbol in &row.symbols {
            spans.entry(symbol).or_default().push((start, exit));
        }
    }
    if let Some(span) = calendar {
        spans
            .entry(super::super::ETF_BENCHMARK)
            .or_default()
            .push(span);
    }
    let mut out = Vec::new();
    for (symbol, mut list) in spans {
        list.sort_unstable();
        let mut merged: Vec<(NaiveDate, NaiveDate)> = Vec::new();
        for (start, end) in list {
            match merged.last_mut() {
                Some(last) if start <= last.1 + chrono::Duration::days(1) => {
                    last.1 = last.1.max(end)
                }
                _ => merged.push((start, end)),
            }
        }
        out.extend(
            merged
                .into_iter()
                .map(|(start, end)| (symbol.to_owned(), start, end)),
        );
    }
    out.sort_unstable();
    out
}

/// 依 `symbol` 在 `windows` 的各段分別還原 `series`（由舊到新、只含這些段內的日線）。
pub(super) fn adjust_windows(
    series: &[PriceBar],
    windows: &[(String, NaiveDate, NaiveDate)],
    symbol: &str,
    events: &[PriceAdjustment],
) -> Vec<PriceBar> {
    let mut out = Vec::with_capacity(series.len());
    for (_, start, end) in windows.iter().filter(|(s, _, _)| s == symbol) {
        let from = series.partition_point(|bar| bar.date < *start);
        let to = series.partition_point(|bar| bar.date <= *end);
        if from < to {
            out.extend(adjust_bars(&series[from..to], events));
        }
    }
    out
}

/// 期間停損用的日線快取：同一次請求的各個換股日偏移共用，只查還沒讀過的股票。
#[derive(Default)]
pub(in super::super) struct BarCache {
    bars: Mutex<HashMap<String, Arc<Vec<PriceBar>>>>,
}

impl BarCache {
    /// `rows` 已滿持有天數的持股在 `from`～`to` 的還原日線；沒讀過的股票一次查回並留在快取。
    /// 查不到日線的股票不在結果中。
    pub(in super::super) async fn load(
        &self,
        rows: &[PeriodRow],
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<HashMap<String, Arc<Vec<PriceBar>>>, sqlx::Error> {
        let mut wanted: HashSet<&str> = rows
            .iter()
            .filter(|row| !row.pending)
            .flat_map(|row| row.symbols.iter().map(String::as_str))
            .collect();
        wanted.insert(super::super::ETF_BENCHMARK);
        // 持鎖查詢：同時跑的偏移等前一個查完再看缺哪些，避免重複讀同一批日線。
        let mut cache = self.bars.lock().await;
        let missing: Vec<String> = wanted
            .iter()
            .filter(|symbol| !cache.contains_key(**symbol))
            .map(|symbol| (*symbol).to_owned())
            .collect();
        if !missing.is_empty() {
            let mut loaded = PgQuoteRepository::new()
                .fetch_adjusted_bars(&missing, from, to)
                .await?;
            for symbol in missing {
                let series = loaded.remove(&symbol).unwrap_or_default();
                cache.insert(symbol, Arc::new(series));
            }
        }
        Ok(wanted
            .into_iter()
            .filter_map(|symbol| {
                cache
                    .get(symbol)
                    .map(|series| (symbol.to_owned(), Arc::clone(series)))
            })
            .collect())
    }
}
