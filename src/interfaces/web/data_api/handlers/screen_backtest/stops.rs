//! 條件選股回測的期間停損：持有期間每天以還原收盤檢查，跌破停損（相對買進價）或移動停利（相對
//! 持有期間最高收盤）就在當天收盤賣出，該檔到下次換股前都放現金。
//!
//! 只檢查已滿持有天數的期數：換股日收盤買進，檢查之後到下一次換股日前一天的收盤（換股日當天本來
//! 就會調整持股，不另外觸發）；最後一期（目前持股）不檢查。觸發後該檔這期的報酬改為買進到出場的
//! 還原報酬，賣出的手續費與證交稅在 [`super::portfolio::trading_costs`] 扣除；下次換股若仍入選，視為
//! 重新買進。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::NaiveDate;
use tokio::sync::Mutex;

use super::PeriodRow;
use crate::domain::quote::technical::{PriceAdjustment, PriceBar, adjust_bars};
use crate::infra::database::repository::quote::PgQuoteRepository;

/// 停損與移動停利門檻（%，正值）；都沒設時不檢查。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct StopRule {
    /// 收盤相對買進價跌幅達這個比例就賣出。
    pub(super) stop_loss: Option<f64>,
    /// 收盤相對持有期間最高收盤（含買進價）跌幅達這個比例就賣出。
    pub(super) trailing_stop: Option<f64>,
}

impl StopRule {
    /// 有設停損或移動停利。
    pub(super) fn is_active(&self) -> bool {
        self.stop_loss.is_some() || self.trailing_stop.is_some()
    }

    /// 依持有期間的還原收盤判斷是否出場：`path[0]` 是買進價，之後依序是可以觸發的交易日收盤。
    /// 觸發時回傳買進到出場當天收盤的報酬（%）；沒觸發或買進價不正回 `None`。
    pub(super) fn exit_return(&self, path: &[f64]) -> Option<f64> {
        let (&entry, rest) = path.split_first()?;
        if entry <= 0.0 {
            return None;
        }
        let mut peak = entry;
        for &close in rest {
            peak = peak.max(close);
            let from_entry = (close / entry - 1.0) * 100.0;
            let from_peak = (close / peak - 1.0) * 100.0;
            if self.stop_loss.is_some_and(|limit| from_entry <= -limit)
                || self.trailing_stop.is_some_and(|limit| from_peak <= -limit)
            {
                return Some(from_entry);
            }
        }
        None
    }
}

/// 一檔持股這期可以觸發的收盤：換股日（含之前最近一根）收盤，接著換股日之後、`end` 之前（不含）的
/// 收盤；`end` 為 `None`（沒有下一期）時取換股日之後 `hold_days` − 1 根。`bars` 由舊到新；沒有換股日
/// 當天或之前的日線為空。
pub(super) fn holding_path(
    bars: &[PriceBar],
    date: NaiveDate,
    end: Option<NaiveDate>,
    hold_days: u16,
) -> Vec<f64> {
    let after = bars.partition_point(|bar| bar.date <= date);
    let Some(entry) = after.checked_sub(1) else {
        return Vec::new();
    };
    let rest = bars[after..]
        .iter()
        .take_while(|bar| end.is_none_or(|end| bar.date < end));
    let rest: Vec<f64> = match end {
        Some(_) => rest.map(|bar| bar.close).collect(),
        None => rest
            .take(usize::from(hold_days.saturating_sub(1)))
            .map(|bar| bar.close)
            .collect(),
    };
    std::iter::once(bars[entry].close).chain(rest).collect()
}

/// 依停損規則改寫各期持股的報酬：觸發的持股報酬改為出場報酬、[`PeriodRow::stopped`] 標記為 true，
/// 平均報酬重算為有報酬持股的等權平均（加權由之後的 [`super::portfolio::weigh`] 處理）。還沒滿持有
/// 天數的期數、沒有報酬或查不到日線的持股不檢查。
pub(super) fn apply_stops(
    rows: &mut [PeriodRow],
    bars: &HashMap<String, Arc<Vec<PriceBar>>>,
    rule: StopRule,
    hold_days: u16,
) {
    if !rule.is_active() {
        return;
    }
    let ends: Vec<Option<NaiveDate>> = (0..rows.len())
        .map(|index| rows.get(index + 1).map(|next| next.date))
        .collect();
    for (row, end) in rows.iter_mut().zip(ends) {
        if row.pending {
            continue;
        }
        let mut stopped = vec![false; row.symbols.len()];
        for (index, symbol) in row.symbols.iter().enumerate() {
            if row.returns.get(index).copied().flatten().is_none() {
                continue;
            }
            let Some(series) = bars.get(symbol) else {
                continue;
            };
            let path = holding_path(series, row.date, end, hold_days);
            if let Some(ret) = rule.exit_return(&path) {
                row.returns[index] = Some(ret);
                stopped[index] = true;
            }
        }
        if stopped.contains(&true) {
            let returns: Vec<f64> = row.returns.iter().flatten().copied().collect();
            row.avg_return =
                (!returns.is_empty()).then(|| returns.iter().sum::<f64>() / returns.len() as f64);
            row.stopped = stopped;
        }
    }
}
/// 以已決定的權重建立同日對齊的策略與 0050 路徑；停損後固定現金價值。
/// 換股成本在期初扣除，停損賣出成本在觸發日扣除，缺日線時標為不完整。
///
/// `open_ratios` 為次日開盤成交（`next_open`）時各持股在進場日、出場日的開盤價／收盤價比（見
/// [`load_open_ratios`]）：路徑從換股日的次一交易日開盤買進起算，出場日以開盤價結算，與該期報酬一致；
/// 缺比值時取 1（沿用收盤），與回測 SQL 相同。收盤成交時為 `None`。
pub(super) fn populate_daily_returns(
    rows: &mut [PeriodRow],
    bars: &HashMap<String, Arc<Vec<PriceBar>>>,
    rule: StopRule,
    fee_discount: f64,
    costs: &[super::portfolio::PeriodCost],
    open_ratios: Option<&OpenRatios>,
) {
    use super::portfolio::{FEE_RATE, TAX_RATE, shares, stopped_value};
    // 0050 作為共用交易日曆；只建一次，避免每一期重掃所有標的多年日線。
    let calendar: std::collections::BTreeSet<_> = match bars
        .get(super::ETF_BENCHMARK)
        .filter(|series| !series.is_empty())
    {
        Some(series) => series.iter().map(|bar| bar.date).collect(),
        None => bars
            .values()
            .flat_map(|series| series.iter().map(|bar| bar.date))
            .collect(),
    };
    for (row, cost) in rows.iter_mut().zip(costs) {
        let Some(exit) = row.exit_date.filter(|_| !row.pending) else {
            continue;
        };
        // 收盤成交從換股日收盤起算；次日開盤成交從次一交易日開盤起算。
        let start = match open_ratios {
            None => row.date,
            Some(_) => match row.entry_date.or_else(|| entry_date(&calendar, row.date)) {
                Some(date) if date <= exit => date,
                _ => {
                    row.daily_complete = false;
                    continue;
                }
            },
        };
        // 次日開盤成交：買進價用進場日開盤，出場日的路徑值用開盤；其餘（含進場日當天）用收盤。
        let ratio = |symbol: &str, date: NaiveDate| {
            open_ratios
                .and_then(|ratios| ratios.get(&(symbol.to_owned(), date)).copied())
                .unwrap_or(1.0)
        };
        let entry_at = |symbol: &str, close: f64| match open_ratios {
            Some(_) => close * ratio(symbol, start),
            None => close,
        };
        let price = |symbol: &str, date: NaiveDate, close: f64| match open_ratios {
            Some(_) if date == exit => close * ratio(symbol, date),
            _ => close,
        };
        // 日期以所有已載入日線聯集對齊，停牌標的沿用最近成交價，不以向量索引代替日期。
        let mut dates: std::collections::BTreeSet<_> = std::iter::once(start)
            .chain(calendar.range(start..=exit).copied())
            .chain(std::iter::once(exit))
            .collect();
        // 核心部位若有停牌或缺日，仍納入衛星持股在該期間的實際成交日。
        for symbol in &row.symbols {
            if let Some(series) = bars.get(symbol) {
                let from = series.partition_point(|bar| bar.date <= start);
                dates.extend(
                    series[from..]
                        .iter()
                        .take_while(|bar| bar.date <= exit)
                        .map(|bar| bar.date),
                );
            }
        }
        row.daily_returns.clear();
        row.daily_net_returns.clear();
        row.etf_daily_returns.clear();
        row.etf_daily_complete = false;
        let weights = shares(row);
        let mut paths = Vec::new();
        row.daily_complete = true;
        let sell_rate = FEE_RATE * fee_discount + TAX_RATE;
        let entry_cost =
            (row.avg_return.unwrap_or(0.0) - cost.net_return - stopped_value(row) * sell_rate)
                .max(0.0);
        for (index, symbol) in row.symbols.iter().enumerate() {
            let Some(series) = bars.get(symbol) else {
                row.daily_complete = false;
                break;
            };
            let entry = series.partition_point(|bar| bar.date <= start);
            if entry == 0 || series[entry - 1].close <= 0.0 {
                row.daily_complete = false;
                break;
            }
            let entry_price = entry_at(symbol, series[entry - 1].close);
            let mut peak = entry_price;
            let mut sold = None;
            let mut path = Vec::new();
            for &date in &dates {
                let at = series.partition_point(|bar| bar.date <= date);
                let close = price(symbol, date, series[at - 1].close);
                peak = peak.max(close);
                let ret = (close / entry_price - 1.0) * 100.0;
                if sold.is_none()
                    && date > start
                    && date < exit
                    && row.stopped.get(index).copied().unwrap_or(false)
                    && (rule.stop_loss.is_some_and(|limit| ret <= -limit)
                        || rule
                            .trailing_stop
                            .is_some_and(|limit| (close / peak - 1.0) * 100.0 <= -limit))
                {
                    sold = Some(ret);
                }
                let value = sold.unwrap_or(ret);
                let fee = sold.map_or(0.0, |ret| {
                    weights[index] * (1.0 + ret / 100.0).max(0.0) * sell_rate
                });
                path.push((weights[index] * value, fee));
            }
            paths.push(path);
        }
        if row.daily_complete {
            for t in 0..dates.len() {
                let gross: f64 = paths.iter().map(|path| path[t].0).sum();
                let fees: f64 = paths.iter().map(|path| path[t].1).sum();
                row.daily_returns.push(gross);
                row.daily_net_returns.push(gross - entry_cost - fees);
            }
        }
        if let Some(series) = bars.get(super::ETF_BENCHMARK) {
            let at = series.partition_point(|bar| bar.date <= start);
            if at > 0 && series[at - 1].close > 0.0 && row.etf_return.is_some() {
                let entry = entry_at(super::ETF_BENCHMARK, series[at - 1].close);
                row.etf_daily_complete = true;
                row.etf_daily_returns = dates
                    .iter()
                    .map(|date| {
                        let at = series.partition_point(|bar| bar.date <= *date);
                        let value = price(super::ETF_BENCHMARK, *date, series[at - 1].close);
                        (value / entry - 1.0) * 100.0
                    })
                    .collect();
            }
        }
    }
}

/// 各股在指定交易日的開盤價／收盤價比，鍵為 `(代號, 日期)`；次日開盤成交的日頻路徑用。
pub(super) type OpenRatios = HashMap<(String, NaiveDate), f64>;

/// 換股日之後的第一個交易日（`calendar` 為 0050 或全部日線的交易日）。
pub(super) fn entry_date(
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
pub(super) async fn load_open_ratios(
    rows: &[PeriodRow],
    bars: &HashMap<String, Arc<Vec<PriceBar>>>,
) -> Result<OpenRatios, sqlx::Error> {
    let calendar: std::collections::BTreeSet<NaiveDate> = bars
        .get(super::ETF_BENCHMARK)
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
            .chain([super::ETF_BENCHMARK])
        {
            points.insert((symbol.to_owned(), entry));
            points.insert((symbol.to_owned(), exit));
        }
    }
    let points: Vec<(String, NaiveDate)> = points.into_iter().collect();
    PgQuoteRepository::new().fetch_open_ratios(&points).await
}

/// 持有期間往前多讀的日曆天：換股日當天停牌時，沿用之前最近的收盤當買進價。
const WINDOW_LOOKBACK_DAYS: i64 = 30;

/// 只算日頻路徑（沒有停損）時的日線：各持股只讀「換股日前 [`WINDOW_LOOKBACK_DAYS`] 天～出場日」，
/// [`super::ETF_BENCHMARK`] 讀整段當交易日曆；同一檔重疊或相鄰的期間先合併成一段。
///
/// 每段各自還原：日頻路徑只比較同一期內的收盤，段內的相對報酬與整段還原相同。
pub(super) async fn load_holding_windows(
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
pub(super) fn holding_windows(rows: &[PeriodRow]) -> Vec<(String, NaiveDate, NaiveDate)> {
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
        spans.entry(super::ETF_BENCHMARK).or_default().push(span);
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
fn adjust_windows(
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
pub(super) struct BarCache {
    bars: Mutex<HashMap<String, Arc<Vec<PriceBar>>>>,
}

impl BarCache {
    /// `rows` 已滿持有天數的持股在 `from`～`to` 的還原日線；沒讀過的股票一次查回並留在快取。
    /// 查不到日線的股票不在結果中。
    pub(super) async fn load(
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
        wanted.insert(super::ETF_BENCHMARK);
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod daily_tests;
