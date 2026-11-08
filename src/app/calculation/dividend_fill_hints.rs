//! 每檔股票的歷年填息提示：近 5 年填息次數與平均天數；沒有自身紀錄時改用同現金殖利率組的
//! 全市場基準（`/market/dividend-fill-stats`，共用 6 小時快取）。
//!
//! 08:00 除權息清單（[`FillHints::text`](crate::app::calculation::dividend_fill_hints::FillHints::text)）與 `/market/dividend-calendar`（[`FillHints::history`](crate::app::calculation::dividend_fill_hints::FillHints::history)、
//! [`FillHints::peer`](crate::app::calculation::dividend_fill_hints::FillHints::peer)）共用。
//!
//! 每檔只查一次近 5 年的除權息與日線（與 `/portfolio/dividend-fill` 的 history 相同規則）；
//! 單檔或全市場基準查詢失敗只記錄警告，那一檔不附文字，不影響提醒本身。

use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::{Days, Local, NaiveDate};
use rust_decimal::Decimal;

use crate::domain::dividend::entity::StockDividendInfo;
use crate::domain::dividend::fill::{self, DividendFillQuery, FillSummary};
use crate::domain::dividend::fill_stats::{self, BucketStats, YieldBucket, bucket_stats};
use crate::infra::cache::dividend_fill_stats;
use crate::infra::database::repository::dividend_fill::PgDividendFillRepository;

/// 自身填息紀錄回看的天數（約 5 年）。
const HISTORY_DAYS: u64 = 5 * 365 + 1;

/// 全市場基準的統計年數；與 [`HISTORY_DAYS`] 一致。
const PEER_YEARS: u8 = fill_stats::MARKET_PEER_YEARS;

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
        self.text_for(
            &stock.stock_symbol,
            stock.cash_dividend,
            stock.closing_price,
        )
    }

    /// 一檔股票要附的填息文字：有自身紀錄寫次數與平均天數，沒有時以現金股利 ÷ 收盤寫同殖利率組中位數；
    /// 都沒有時為 `None`。
    #[must_use]
    pub fn text_for(
        &self,
        symbol: &str,
        cash_dividend: Decimal,
        closing_price: Decimal,
    ) -> Option<String> {
        if let Some(history) = self.history(symbol) {
            let average = history
                .average_trading_days
                .map_or_else(String::new, |days| format!("、平均 {days} 日"));
            return Some(format!(
                "近 5 年填息 {}／{} 次{average}",
                history.filled, history.events
            ));
        }
        let peer = self.peer(cash_dividend, closing_price)?;
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

/// 重新查詢今天的全市場填息基準（[`PEER_YEARS`] 年）並覆蓋共用快取。
///
/// 啟動時與每 6 小時由排程呼叫，讓股利預估、持股填息與除權息提醒直接命中快取，
/// 不必在請求中現查（現查遇上 pi 高負載時會超過 Go 端逾時）。換日後快取鍵跟著換，
/// 所以排程的第一次在 00:05。
///
/// # Errors
///
/// 查詢失敗時回傳錯誤；原有快取保留，請求仍可讀舊值或自行現查。
pub async fn refresh_market_peers() -> Result<()> {
    let today = Local::now().date_naive();
    let samples = dividend_fill_stats::refresh_market_fill_samples_for(
        &PgDividendFillRepository::new(),
        today,
        PEER_YEARS,
    )
    .await
    .context("Failed to refresh market dividend fill peers")?;
    tracing::info!("全市場填息基準已更新: today={today}, samples={samples}");
    Ok(())
}

#[cfg(test)]
mod tests;
