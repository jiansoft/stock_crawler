//! 區間突破與假突破訊號的歷史回補。
//!
//! 逐日重算全部訊號（`manual_backfill` 的 `test_backfill_technical_signals`）每天要讀全市場 361 根日線，
//! 回補多年要好幾個小時。區間突破只看最近 31 根，因此改成每檔讀一次整段日線與成交量，在記憶體逐日判斷，
//! 最後只覆蓋 `range_breakout` 來源，其他訊號不動。
//!
//! 還原權值以整段的最後一天為基準；區間突破與假突破只比較 41 根之內的價格比例，基準日不同只差一個
//! 共同倍數，結果與每日計算（以當天為基準）相同。

use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use futures::{StreamExt, stream};

use super::{CONCURRENCY, PgQuoteRepository};
use crate::domain::quote::technical::{
    self, PriceAdjustment, PriceBarRepository, RANGE_BREAKOUT_INDICATOR, StockTechnicalSignal,
    TechnicalSignalRepository, breakout::FAILED_BREAKOUT_BARS,
};

/// 回補 `from`～`to`（含）的區間突破與假突破訊號，覆蓋這段期間原有的 `range_breakout` 訊號，回傳寫入筆數。
///
/// # Errors
///
/// 讀取股票清單、除權息事件、任一檔日線或成交量、或寫入失敗時回傳錯誤；讀取失敗時不寫入。
pub async fn backfill_range_breakout(from: NaiveDate, to: NaiveDate) -> Result<usize> {
    backfill_with(&PgQuoteRepository::new(), from, to).await
}

/// [`backfill_range_breakout`] 的本體，倉儲可替換以便測試。
pub(crate) async fn backfill_with<R>(repo: &R, from: NaiveDate, to: NaiveDate) -> Result<usize>
where
    R: PriceBarRepository + TechnicalSignalRepository,
{
    anyhow::ensure!(from <= to, "起日 {from} 晚於迄日 {to}");
    let symbols = repo.fetch_traded_symbols(from, to).await?;
    let adjustments = repo
        .fetch_price_adjustments(&symbols)
        .await
        .context("讀取除權息事件失敗")?;
    // 日曆天數一定不少於交易日數；多讀判斷所需的根數，讓 `from` 當天也有完整的整理區間與假突破回看。
    let limit = (to - from).num_days() + 1 + FAILED_BREAKOUT_BARS as i64;
    let results: Vec<_> = stream::iter(symbols)
        .map(|symbol| async move {
            let fetched = futures::join!(
                repo.fetch_price_bars(&symbol, Some(to), limit),
                repo.fetch_volumes(&symbol, Some(to), limit),
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
        signals.extend(symbol_signals(&symbol, from, &bars, &volumes, events));
    }
    signals.sort_by(|a, b| (a.date, &a.stock_symbol).cmp(&(b.date, &b.stock_symbol)));
    repo.replace_indicator_signals(RANGE_BREAKOUT_INDICATOR, from, to, &signals)
        .await?;
    Ok(signals.len())
}

/// 單一股票在 `from` 起每個交易日的區間突破與假突破訊號。`bars` 與 `volumes` 由舊到新；成交量依日期對齊，
/// 缺成交量的日子由判斷函式略過。
fn symbol_signals(
    symbol: &str,
    from: NaiveDate,
    bars: &[technical::PriceBar],
    volumes: &[technical::VolumeBar],
    adjustments: &[PriceAdjustment],
) -> Vec<StockTechnicalSignal> {
    let bars = technical::adjust_bars(bars, adjustments);
    let volume_index: HashMap<NaiveDate, usize> = volumes
        .iter()
        .enumerate()
        .map(|(index, bar)| (bar.date, index))
        .collect();
    let mut out = Vec::new();
    for (index, bar) in bars.iter().enumerate() {
        if bar.date < from {
            continue;
        }
        let Some(&volume_end) = volume_index.get(&bar.date) else {
            continue;
        };
        let signals = technical::range_breakout_signals(&bars[..=index], &volumes[..=volume_end]);
        for signal in signals {
            out.push(StockTechnicalSignal {
                date: bar.date,
                stock_symbol: symbol.to_owned(),
                signal,
            });
        }
    }
    out
}
