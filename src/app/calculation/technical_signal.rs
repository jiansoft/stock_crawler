//! 收盤後全市場技術訊號計算。
//!
//! 對當日有成交的上市櫃股票逐檔讀取日線、還原權值後計算技術指標，把最新一日的交叉、站上或跌破均線、
//! 超買超賣、52 週新高新低與區間突破寫入 `daily_technical_signal`，供 `/market/signals` 查詢與
//! 持股訊號通知使用。規則都在 [`crate::domain::quote::technical`]，與 `/stocks/{symbol}/technicals`
//! 同一套計算，結果一致。區間突破的歷史回補見
//! [`backfill_range_breakout`](crate::app::calculation::technical_signal::backfill_range_breakout)。

use anyhow::{Context, Result};
use chrono::NaiveDate;
use futures::{StreamExt, stream};

use crate::domain::quote::technical::{
    self, PriceAdjustment, PriceBar, PriceBarRepository, StockTechnicalSignal,
    TechnicalSignalRepository, VolumeBar, WARMUP_BARS, breakout::FAILED_BREAKOUT_VOLUME_BARS,
};
use crate::infra::database::repository::quote::PgQuoteRepository;

mod range_breakout;

pub use self::range_breakout::backfill_range_breakout;

/// 同時讀取的股票數；每檔兩個索引查詢（日線、成交量），不需要更多連線。
const CONCURRENCY: usize = 4;

/// 計算並覆蓋指定交易日的全市場技術訊號，回傳寫入的訊號筆數。
///
/// # Errors
///
/// 查詢股票清單、除權息事件、任一檔日線讀取或寫入失敗時回傳錯誤；讀取失敗時不寫入，
/// 保留資料表原本的內容。
pub async fn calculate_technical_signals(date: NaiveDate) -> Result<usize> {
    compute_and_save(&PgQuoteRepository::new(), date).await
}

/// [`calculate_technical_signals`] 的本體，倉儲可替換以便測試。
pub(crate) async fn compute_and_save<R>(repo: &R, date: NaiveDate) -> Result<usize>
where
    R: PriceBarRepository + TechnicalSignalRepository,
{
    let symbols = repo.fetch_signal_symbols(date).await?;
    let adjustments = repo
        .fetch_price_adjustments(&symbols)
        .await
        .context("讀取除權息事件失敗")?;
    // 年線需要 240 日、52 週新高新低需要前 364 天，再加上前一日比較交叉。
    let limit = WARMUP_BARS as i64 + 1;
    type Fetched = (Result<Vec<PriceBar>>, Result<Vec<VolumeBar>>);
    let results: Vec<(String, Fetched)> = stream::iter(symbols)
        .map(|symbol| async move {
            let fetched = futures::join!(
                repo.fetch_price_bars(&symbol, Some(date), limit),
                repo.fetch_volumes(&symbol, Some(date), FAILED_BREAKOUT_VOLUME_BARS as i64),
            );
            (symbol, fetched)
        })
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;

    let mut signals = Vec::new();
    for (symbol, (bars, volumes)) in results {
        let bars = bars.with_context(|| format!("讀取 {symbol} 日線失敗"))?;
        let volumes = volumes.with_context(|| format!("讀取 {symbol} 成交量失敗"))?;
        let events = adjustments.get(&symbol).map_or(&[][..], Vec::as_slice);
        signals.extend(signals_for(&symbol, date, &bars, &volumes, events));
    }
    signals.sort_by(|a, b| {
        a.stock_symbol
            .cmp(&b.stock_symbol)
            .then_with(|| a.signal.indicator.cmp(b.signal.indicator))
    });
    repo.replace_signals(date, &signals).await?;
    Ok(signals.len())
}

/// 單一股票在 `date` 的訊號（日線先以 `adjustments` 還原權值）；最後一根日線不是 `date`
/// （當日沒有成交）時沒有訊號。區間突破與假突破另外需要同一批交易日的成交量 `volumes`。
pub(crate) fn signals_for(
    symbol: &str,
    date: NaiveDate,
    bars: &[PriceBar],
    volumes: &[VolumeBar],
    adjustments: &[PriceAdjustment],
) -> Vec<StockTechnicalSignal> {
    if bars.last().is_none_or(|bar| bar.date != date) {
        return Vec::new();
    }
    let bars = technical::adjust_bars(bars, adjustments);
    let bars = bars.as_slice();
    let points = technical::compute(bars);
    let Some((last, rest)) = points.split_last() else {
        return Vec::new();
    };
    technical::signals(rest.last(), last)
        .into_iter()
        .chain(technical::fifty_two_week_signals(bars))
        .chain(technical::range_breakout_signals(bars, volumes))
        .map(|signal| StockTechnicalSignal {
            date,
            stock_symbol: symbol.to_owned(),
            signal,
        })
        .collect()
}

#[cfg(test)]
mod tests;
