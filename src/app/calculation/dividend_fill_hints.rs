//! 每檔股票的歷年填息提示：近 5 年填息次數與平均天數；沒有自身紀錄時改用同現金殖利率組的
//! 全市場基準（`/market/dividend-fill-stats`，共用 6 小時快取）。
//!
//! 08:00 除權息清單（[`FillHints::text`](crate::app::calculation::dividend_fill_hints::FillHints::text)）與 `/market/dividend-calendar`（[`FillHints::history`](crate::app::calculation::dividend_fill_hints::FillHints::history)、
//! [`FillHints::peer`](crate::app::calculation::dividend_fill_hints::FillHints::peer)）共用。
//!
//! 每檔只查一次近 5 年的除權息與日線（與 `/portfolio/dividend-fill` 的 history 相同規則）；
//! 單檔或全市場基準查詢失敗只記錄警告，那一檔不附文字，不影響提醒本身。

use std::collections::HashMap;

use chrono::{Days, NaiveDate};
use rust_decimal::Decimal;

use crate::domain::dividend::entity::StockDividendInfo;
use crate::domain::dividend::fill::{self, DividendFillQuery, FillSummary};
use crate::domain::dividend::fill_stats::{BucketStats, YieldBucket, bucket_stats};
use crate::infra::cache::dividend_fill_stats;
use crate::infra::database::repository::dividend_fill::PgDividendFillRepository;

/// 自身填息紀錄回看的天數（約 5 年）。
const HISTORY_DAYS: u64 = 5 * 365 + 1;

/// 全市場基準的統計年數；與 [`HISTORY_DAYS`] 一致。
const PEER_YEARS: u8 = 5;

/// 一批股票的歷年填息統計與全市場基準。
#[derive(Debug, Default)]
pub struct FillHints {
    history: HashMap<String, FillSummary>,
    peers: Vec<BucketStats>,
}

impl FillHints {
    /// 查詢 `symbols` 近 5 年的填息統計與全市場基準。
    pub async fn load(symbols: &[String], today: NaiveDate) -> Self {
        let repository = PgDividendFillRepository::new();
        let peers = match dividend_fill_stats::market_fill_samples_for(
            &repository,
            today,
            PEER_YEARS,
        )
        .await
        {
            Ok(samples) => bucket_stats(&samples),
            Err(why) => {
                tracing::warn!("除權息提醒的全市場填息基準查詢失敗: error={why:#}");
                Vec::new()
            }
        };
        Self {
            history: load_history(&repository, symbols, today).await,
            peers,
        }
    }

    /// 測試用：直接帶入填息統計與全市場基準。
    #[cfg(test)]
    pub(crate) fn for_test(history: HashMap<String, FillSummary>, peers: Vec<BucketStats>) -> Self {
        Self { history, peers }
    }

    /// 該股近 5 年的填息統計；查詢失敗或期間內沒有除權息時為 `None`。
    #[must_use]
    pub fn history(&self, symbol: &str) -> Option<&FillSummary> {
        self.history
            .get(symbol)
            .filter(|history| history.events > 0)
    }

    /// 以現金股利與收盤價算出殖利率組別的全市場基準；沒有現金股利、收盤不大於 0 或基準查詢失敗時為 `None`。
    #[must_use]
    pub fn peer(&self, cash_dividend: Decimal, closing_price: Decimal) -> Option<&BucketStats> {
        let bucket = YieldBucket::of(cash_dividend, closing_price)?;
        self.peers.iter().find(|stats| stats.bucket == bucket)
    }

    /// 除權息清單這一檔要附的文字；沒有自身紀錄也沒有基準時為 `None`。
    #[must_use]
    pub fn text(&self, stock: &StockDividendInfo) -> Option<String> {
        if let Some(history) = self.history(&stock.stock_symbol) {
            let average = history
                .average_trading_days
                .map_or_else(String::new, |days| format!("、平均 {days} 日"));
            return Some(format!(
                "近 5 年填息 {}／{} 次{average}",
                history.filled, history.events
            ));
        }
        let peer = self.peer(stock.cash_dividend, stock.closing_price)?;
        let median = peer.median_trading_days?;
        Some(format!(
            "同殖利率組（{}）填息中位數 {median} 日",
            peer.bucket.label()
        ))
    }
}

/// 每檔近 5 年的填息統計；同一代號只查一次，失敗的略過。
async fn load_history(
    repository: &impl DividendFillQuery,
    symbols: &[String],
    today: NaiveDate,
) -> HashMap<String, FillSummary> {
    let since = today - Days::new(HISTORY_DAYS);
    let mut history = HashMap::new();
    for symbol in symbols {
        if history.contains_key(symbol) {
            continue;
        }
        match fill::recent_outcomes(repository, symbol, since, today).await {
            Ok(outcomes) => {
                history.insert(symbol.clone(), fill::summarize(&outcomes));
            }
            Err(why) => {
                tracing::warn!("除權息提醒的填息紀錄查詢失敗: stock_symbol={symbol}, error={why:#}")
            }
        }
    }
    history
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::domain::dividend::fill::{DailyClose, ExRightEvent};
    use crate::domain::dividend::fill_stats::FillSample;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).expect("日期應合法")
    }

    /// 2330 9 月除息第 2 天填息；9999 查詢失敗；其他代號沒有除權息。
    struct FakeFills;

    #[async_trait]
    impl DividendFillQuery for FakeFills {
        async fn fetch_latest_quote_date(&self, _symbol: &str) -> Result<Option<NaiveDate>> {
            unreachable!("不需要最新收盤日")
        }

        async fn fetch_ex_right_events(&self, symbol: &str) -> Result<Vec<ExRightEvent>> {
            match symbol {
                "2330" => Ok(vec![ExRightEvent {
                    ex_date: day(9, 15),
                    cash_dividend: dec!(5),
                    stock_dividend: Decimal::ZERO,
                }]),
                "9999" => Err(anyhow!("db down")),
                _ => Ok(Vec::new()),
            }
        }

        async fn fetch_daily_closes(
            &self,
            _symbol: &str,
            _since: NaiveDate,
        ) -> Result<Vec<DailyClose>> {
            let close = |date, close| DailyClose { date, close };
            Ok(vec![
                close(day(9, 12), dec!(100)),
                close(day(9, 15), dec!(97)),
                close(day(9, 16), dec!(101)),
            ])
        }
    }

    fn stock(symbol: &str, cash: Decimal, closing_price: Decimal) -> StockDividendInfo {
        StockDividendInfo {
            stock_symbol: symbol.to_owned(),
            name: symbol.to_owned(),
            stock_industry_id: 1,
            cash_dividend: cash,
            stock_dividend: Decimal::ZERO,
            sum: cash,
            closing_price,
            dividend_yield: Decimal::ZERO,
            cash_dividend_yield: Decimal::ZERO,
            is_cash_ex_dividend_on_date: true,
            is_stock_ex_dividend_on_date: false,
            quarter: String::new(),
        }
    }

    /// 有自身紀錄附次數與平均；沒有時附同殖利率組中位數；都沒有時不附；查詢失敗的略過、重複代號只查一次。
    #[tokio::test]
    async fn text_prefers_own_history_then_peer_median() {
        let history = load_history(
            &FakeFills,
            &["2330", "2330", "1101", "9999"].map(str::to_owned),
            day(11, 2),
        )
        .await;
        assert_eq!(history.len(), 2, "9999 查詢失敗略過");
        let hints = FillHints {
            history,
            peers: bucket_stats(&[FillSample {
                bucket: YieldBucket::From3To5,
                trading_days_to_fill: Some(13),
                trading_days_observed: 120,
            }]),
        };
        assert_eq!(
            hints.text(&stock("2330", dec!(5), dec!(500))).as_deref(),
            Some("近 5 年填息 1／1 次、平均 2 日")
        );
        assert_eq!(
            hints.text(&stock("1101", dec!(1), dec!(25))).as_deref(),
            Some("同殖利率組（3～5%）填息中位數 13 日"),
            "1101 沒有除權息紀錄，1 ÷ 25 = 4%"
        );
        assert_eq!(
            hints.text(&stock("1101", dec!(1), dec!(10))),
            None,
            "10% 組沒有基準"
        );
        assert_eq!(hints.text(&stock("9999", Decimal::ZERO, dec!(10))), None);
    }
}
