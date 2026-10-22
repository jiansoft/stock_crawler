//! # ETF 含息報酬
//!
//! 以還原日線（除權息、分割已調整）計算全部上市櫃 ETF 的期間報酬與近 1 年風險指標，
//! 寫入 `etf_performance`；基準為 0050。日線分批讀取，避免一次把 10 年 × 數百檔載入記憶體。
//!
//! 同一批也以 `etf_daily_nav` 的正式淨值計算淨值含息報酬（`nav_ret_*`）：淨值只以現金配息還原，
//! 不套用分割、股票股利——MoneyDJ 回補的歷史淨值已依分割調整，再套一次會重複。

use std::collections::HashMap;

use anyhow::Result;
use chrono::{Days, Local};

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
        let navs = repo.nav_series(chunk, from).await?;
        let adjustments = quotes.fetch_price_adjustments(chunk).await?;
        let mut computed = compute_all(chunk, &bars, &benchmark);
        attach_nav_returns(&mut computed, &navs, &adjustments);
        performances.extend(computed);
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
    fn nav_returns_reinvest_cash_dividends_and_ignore_splits() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 1).expect("日期合法");
        // 淨值 100 → 第 3 天除息 2 元後 99：配息以除息參考價 98 再投入，含息報酬 = 99 / 98 − 1。
        let mut navs = HashMap::new();
        navs.insert("0056".to_string(), bars_from(start, &[100.0, 100.0, 99.0]));
        let mut adjustments = HashMap::new();
        adjustments.insert(
            "0056".to_string(),
            vec![
                PriceAdjustment {
                    date: start + Days::new(2),
                    cash_dividend: 2.0,
                    stock_dividend: 0.0,
                    share_ratio: 1.0,
                },
                // 分割：淨值歷史已調整過，不可再套。
                PriceAdjustment {
                    date: start + Days::new(1),
                    cash_dividend: 0.0,
                    stock_dividend: 0.0,
                    share_ratio: 4.0,
                },
            ],
        );
        let mut performances = vec![
            EtfPerformance {
                stock_symbol: "0056".to_string(),
                ..Default::default()
            },
            EtfPerformance {
                stock_symbol: "00999".to_string(),
                ..Default::default()
            },
        ];
        attach_nav_returns(&mut performances, &navs, &adjustments);
        let nav = performances[0].nav_returns.as_ref().expect("有淨值");
        assert_eq!(nav.calc_date, start + Days::new(2));
        let one_week = nav.ret_1w;
        assert_eq!(one_week, None, "不足一週");
        assert!(performances[1].nav_returns.is_none());

        // 配息在分割之後，金額不換算。
        let adjusted = adjust_bars(&navs["0056"], &cash_only(&adjustments["0056"]));
        let total = (adjusted[2].close / adjusted[0].close - 1.0) * 100.0;
        let expected = (99.0 / 98.0 - 1.0) * 100.0;
        assert!(
            (total - expected).abs() < 1e-9,
            "含息報酬應為 {expected}%，實際 {total}"
        );
    }

    #[test]
    fn cash_before_split_is_scaled_to_current_units() {
        let day = |d| NaiveDate::from_ymd_opt(2025, 6, d).expect("日期合法");
        let event = |date, cash, ratio| PriceAdjustment {
            date,
            cash_dividend: cash,
            stock_dividend: 0.0,
            share_ratio: ratio,
        };
        let cash = cash_only(&[
            event(day(1), 4.0, 1.0),
            event(day(18), 0.0, 4.0),
            event(day(25), 1.0, 1.0),
        ]);
        let amounts: Vec<(NaiveDate, f64, f64)> = cash
            .iter()
            .map(|e| (e.date, e.cash_dividend, e.share_ratio))
            .collect();
        assert_eq!(amounts, vec![(day(1), 1.0, 1.0), (day(25), 1.0, 1.0)]);
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
