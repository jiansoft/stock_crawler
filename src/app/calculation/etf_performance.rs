//! # ETF 含息報酬
//!
//! 以還原日線（除權息、分割已調整）計算全部上市櫃 ETF 的期間報酬與近 1 年風險指標，
//! 寫入 `etf_performance`；基準為 0050。日線分批讀取，避免一次把 10 年 × 數百檔載入記憶體。

use std::collections::HashMap;

use anyhow::Result;
use chrono::{Days, Local};

use crate::{
    domain::{
        etf::{EtfPerformance, EtfRepository, compute_performance},
        quote::technical::PriceBar,
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
    let repo = PgEtfRepository::new();
    let symbols: Vec<String> = repo
        .listed_etfs()
        .await?
        .into_iter()
        .map(|(symbol, _)| symbol)
        .collect();
    let today = Local::now().date_naive();
    let from = today - Days::new(HISTORY_DAYS);
    let quotes = PgQuoteRepository::new();

    let benchmark = quotes
        .fetch_adjusted_bars(&[BENCHMARK.to_string()], from, today)
        .await?
        .remove(BENCHMARK)
        .unwrap_or_default();
    let mut performances = Vec::with_capacity(symbols.len());
    for chunk in symbols.chunks(CHUNK) {
        let bars = quotes.fetch_adjusted_bars(chunk, from, today).await?;
        performances.extend(compute_all(chunk, &bars, &benchmark));
    }
    let written = repo.save_performances(&performances).await?;
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

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    /// 測試用：建立一段日線。
    fn bars_from(start: NaiveDate, closes: &[f64]) -> Vec<PriceBar> {
        closes
            .iter()
            .enumerate()
            .map(|(i, close)| PriceBar {
                date: start + Days::new(i as u64),
                close: *close,
                high: *close,
                low: *close,
            })
            .collect()
    }

    #[test]
    fn compute_all_skips_symbols_without_bars() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 1).expect("日期合法");
        let mut bars = HashMap::new();
        bars.insert("0050".to_string(), bars_from(start, &[100.0, 101.0, 102.0]));
        bars.insert("00999".to_string(), Vec::new());
        let symbols = ["0050".to_string(), "00999".to_string(), "00998".to_string()];
        let result = compute_all(&symbols, &bars, &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].stock_symbol, "0050");
        assert_eq!(result[0].calc_date, Some(start + Days::new(2)));
    }
}
