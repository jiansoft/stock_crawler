//! # ETF 含息報酬
//!
//! 以還原日線（除權息、分割已調整）計算全部上市櫃 ETF 的期間報酬與近 1 年風險指標，
//! 寫入 `etf_performance`；基準為 0050。日線分批讀取，避免一次把 10 年 × 數百檔載入記憶體。
//!
//! 同一批也以 `etf_daily_nav` 的正式淨值計算淨值含息報酬（`nav_ret_*`）：淨值只以現金配息還原，
//! 不套用分割、股票股利——MoneyDJ 回補的歷史淨值已依分割調整，再套一次會重複。

use std::collections::HashMap;

use anyhow::Result;
use chrono::{Days, Local, NaiveDate};

use crate::{
    domain::{
        etf::{EtfPerformance, EtfRepository, compute_nav_returns, compute_performance},
        quote::technical::{PriceAdjustment, PriceBar, PriceBarRepository, adjust_bars},
    },
    infra::database::repository::{etf::PgEtfRepository, quote::PgQuoteRepository},
};

/// 績效的基準 ETF。
pub const BENCHMARK: &str = "0050";
/// 每批讀取的 ETF 檔數。
const CHUNK: usize = 40;
/// 最長期間 10 年，多抓幾天讓期初能落在目標日或之前。
const HISTORY_DAYS: u64 = 10 * 366 + 10;

/// 重算全部 ETF 的績效並寫入，回傳寫入列數。
pub async fn execute() -> Result<u64> {
    run(Local::now().date_naive()).await
}

/// [`execute`] 的本體：以 `today` 為期末，讀近 10 年還原日線計算績效。
async fn run(today: NaiveDate) -> Result<u64> {
    let repo = PgEtfRepository::new();
    let symbols: Vec<String> = repo
        .listed_etfs()
        .await?
        .into_iter()
        .map(|(symbol, _)| symbol)
        .collect();
    let from = today - Days::new(HISTORY_DAYS);
    let quotes = PgQuoteRepository::new();

    let benchmark = quotes
        .fetch_adjusted_bars(&[BENCHMARK.to_string()], from, today)
        .await?
        .remove(BENCHMARK)
        .unwrap_or_default();
    let mut performances = Vec::with_capacity(symbols.len());
    let mut tracking = Vec::new();
    for chunk in symbols.chunks(CHUNK) {
        let bars = quotes.fetch_adjusted_bars(chunk, from, today).await?;
        // 基準缺資料時不計算追蹤誤差，避免以空結果覆蓋前一次的有效值。
        for symbol in chunk.iter().filter(|_| !benchmark.is_empty()) {
            if let Some(series) = bars.get(symbol).filter(|series| !series.is_empty()) {
                tracking.push((
                    symbol.clone(),
                    series.last().expect("非空").date,
                    crate::domain::etf::tracking::compute(series, &benchmark),
                ));
            }
        }
        let navs = repo.nav_series(chunk, from).await?;
        let adjustments = quotes.fetch_price_adjustments(chunk).await?;
        let mut computed = compute_all(chunk, &bars, &benchmark);
        attach_nav_returns(&mut computed, &navs, &adjustments);
        performances.extend(computed);
    }
    let written = repo.save_performances(&performances).await?;
    // 追蹤誤差是附加指標，寫入失敗不可中斷後續的快照刷新。
    if benchmark.is_empty() {
        tracing::warn!("基準 {BENCHMARK} 無還原日線，略過追蹤誤差寫入");
    } else if let Err(why) =
        crate::infra::database::repository::etf::tracking::save(&tracking).await
    {
        tracing::warn!("ETF 追蹤誤差寫入失敗，略過: {why:#}");
    }
    tracing::info!(
        "ETF 績效計算完成: etfs={}, computed={}, written={written}",
        symbols.len(),
        performances.len()
    );
    Ok(written)
}

/// 依代號順序計算有日線的 ETF；沒有任何成交的略過。
fn compute_all(
    symbols: &[String],
    bars: &HashMap<String, Vec<PriceBar>>,
    benchmark: &[PriceBar],
) -> Vec<EtfPerformance> {
    symbols
        .iter()
        .filter_map(|symbol| {
            let series = bars.get(symbol).filter(|series| !series.is_empty())?;
            Some(compute_performance(symbol, series, benchmark))
        })
        .collect()
}

/// 為每檔加上淨值含息報酬；沒有淨值歷史的維持 `None`。
fn attach_nav_returns(
    performances: &mut [EtfPerformance],
    navs: &HashMap<String, Vec<PriceBar>>,
    adjustments: &HashMap<String, Vec<PriceAdjustment>>,
) {
    for perf in performances {
        let Some(series) = navs.get(&perf.stock_symbol) else {
            continue;
        };
        let cash = adjustments
            .get(&perf.stock_symbol)
            .map(|events| cash_only(events))
            .unwrap_or_default();
        perf.nav_returns = compute_nav_returns(&adjust_bars(series, &cash));
    }
}

/// 只留現金配息，股票股利與股數變動一律不調整（理由見模組說明）。
///
/// 淨值序列已換算成最新的股數基準，分割前的配息卻是當時每單位的金額，
/// 因此要除以之後各次股數變動的比例（0050 在 2025 年 1 拆 4，之前的配息要除以 4）。
fn cash_only(events: &[PriceAdjustment]) -> Vec<PriceAdjustment> {
    events
        .iter()
        .filter(|event| event.cash_dividend > 0.0)
        .map(|event| {
            let later_ratio: f64 = events
                .iter()
                .filter(|split| split.date > event.date && split.share_ratio > 0.0)
                .map(|split| split.share_ratio)
                .product();
            PriceAdjustment {
                cash_dividend: event.cash_dividend / later_ratio,
                stock_dividend: 0.0,
                share_ratio: 1.0,
                ..*event
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
