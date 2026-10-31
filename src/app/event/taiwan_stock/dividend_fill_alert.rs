//! # 持股填息通知
//!
//! 每個交易日收盤資料確認完整後，檢查持股（含 ETF）近 [`LOOKBACK_DAYS`] 天內的每次除權息，發一則
//! Telegram，包含兩段（都沒有就不發）：
//!
//! - **填息**：收盤價**今天**第一次回到填息目標價。填息規則與 `/stocks/{symbol}/dividend-fill` 相同
//!   （[`fill::fill_outcome`]）。
//! - **填息落後**：尚未填息、且已過交易日數今天**第一次**超過基準：該股近 5 年的平均填息天數；
//!   沒有自身紀錄時改用同現金殖利率組的全市場中位數（`/market/dividend-fill-stats`）。
//!
//! 兩段都以「今天剛好發生」判斷（填息日是今天、已過天數剛好等於基準取整數加 1），同一次除權息
//! 各只會出現一次，不需另外記錄已通知。今天休市（最新收盤不是今天）時不會有任何一筆成立。
//! 單檔查詢失敗只記錄警告、略過該檔；全市場基準查詢失敗時只用自身紀錄。
//!
//! 落後只是陳述事實：2026-10 研究（T14）顯示遲遲未填息的股票之後多半落後大盤，不是加碼理由。

use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Days, Local, NaiveDate};
use rust_decimal::Decimal;

use super::{format_decimal_with_commas, holdings};
use crate::{
    core::{alert, util::text},
    domain::dividend::{
        fill::{self, DividendFillQuery, FillOutcome, FillSummary},
        fill_stats::{BucketStats, YieldBucket, bucket_stats},
    },
    infra::{
        cache::dividend_fill_stats, database::repository::dividend_fill::PgDividendFillRepository,
    },
};

/// 只看除權息日在今天往前這麼多天內的事件；超過一年還沒填息的不再追蹤。
const LOOKBACK_DAYS: u64 = 365;

/// 自身填息紀錄回看的天數（約 5 年），與 `/portfolio/dividend-fill` 的 history 相同。
const HISTORY_DAYS: u64 = 5 * 365 + 1;

/// 全市場基準的統計年數；與 [`HISTORY_DAYS`] 一致。
const PEER_YEARS: u8 = 5;

/// 落後段的附註（T14：2019 年起上市櫃普通股純現金除息）。
const LAG_FOOTNOTE: &str =
    "註：2019 年起統計，除息 60 個交易日仍未填息者，之後 120 個交易日平均落後大盤約 7%，僅供參考。";

/// 判斷填息落後的基準天數。
#[derive(Debug, Clone, Copy, PartialEq)]
enum Baseline {
    /// 該股近 5 年已填息事件的平均填息交易日。
    Own(f64),
    /// 沒有自身紀錄時，同現金殖利率組的全市場填息中位數。
    Peer { bucket: YieldBucket, median: f64 },
}

impl Baseline {
    fn days(self) -> f64 {
        match self {
            Self::Own(days) | Self::Peer { median: days, .. } => days,
        }
    }

    fn text(self) -> String {
        match self {
            Self::Own(days) => format!("近 5 年平均 {days} 日"),
            Self::Peer { bucket, median } => {
                format!("同殖利率組（{}）全市場中位數 {median} 日", bucket.label())
            }
        }
    }
}

/// 今天第一次超過基準、仍未填息的一次除權息。
#[derive(Debug, Clone, PartialEq)]
struct Lagging {
    symbol: String,
    outcome: FillOutcome,
    baseline: Baseline,
}

/// 今天要通知的事件：填息與填息落後。
#[derive(Debug, Default)]
struct Review {
    filled: Vec<(String, FillOutcome)>,
    lagging: Vec<Lagging>,
}

/// 排程入口：通知今天填息與今天第一次落後基準的持股。
///
/// # Errors
///
/// 查詢持股失敗時回傳錯誤。
pub async fn execute() -> Result<()> {
    let symbols = holdings::active_holding_symbols()
        .await
        .context("fetch active holdings for dividend fill notification failed")?;
    let today = Local::now().date_naive();
    let repository = PgDividendFillRepository::new();
    let peers =
        match dividend_fill_stats::market_fill_samples_for(&repository, today, PEER_YEARS).await {
            Ok(samples) => bucket_stats(&samples),
            Err(why) => {
                tracing::warn!("持股填息通知的全市場基準查詢失敗，只用自身紀錄: error={why:#}");
                Vec::new()
            }
        };
    let review = review(&repository, &symbols, &peers, today).await;
    tracing::info!(
        "持股填息通知: holdings={}, filled={}, lagging={}",
        symbols.len(),
        review.filled.len(),
        review.lagging.len()
    );
    if let Some(message) = build_message(today, &review, holdings::stock_name) {
        alert::send_message(&message).await;
    }
    Ok(())
}

/// 持股近 [`LOOKBACK_DAYS`] 天內今天填息與今天第一次落後的除權息，依代號、除權息日排序；
/// 單檔失敗只記錄警告。每檔只查一次近 5 年的除權息與日線，自身基準從中算出。
async fn review(
    repository: &impl DividendFillQuery,
    symbols: impl IntoIterator<Item = &String>,
    peers: &[BucketStats],
    today: NaiveDate,
) -> Review {
    let since = today - Days::new(LOOKBACK_DAYS);
    let history_since = today - Days::new(HISTORY_DAYS);
    let mut review = Review::default();
    for symbol in symbols {
        let outcomes = match fill::recent_outcomes(repository, symbol, history_since, today).await {
            Ok(outcomes) => outcomes,
            Err(why) => {
                tracing::warn!("持股填息檢查失敗: stock_symbol={symbol}, error={why:#}");
                continue;
            }
        };
        let history = fill::summarize(&outcomes);
        let recent = outcomes
            .into_iter()
            .filter(|outcome| outcome.event.ex_date > since);
        for outcome in recent {
            if outcome.fill_date == Some(today) {
                review.filled.push((symbol.clone(), outcome));
            } else if let Some(baseline) = lag_baseline(&outcome, &history, peers, today) {
                review.lagging.push(Lagging {
                    symbol: symbol.clone(),
                    outcome,
                    baseline,
                });
            }
        }
    }
    review
}

/// 今天（最新收盤是 `today`）仍未填息、且已過交易日數第一次超過基準時回傳該基準。
///
/// 已過交易日數每個交易日加 1，「第一次超過」就是剛好等於 `floor(基準) + 1` 的那一天。
/// 基準先用自身近 5 年的平均填息天數（`history`），沒有已填息紀錄時用同殖利率組的全市場中位數；
/// 兩者都沒有（例如只有股票股利、或基準查詢失敗）時不判斷。
fn lag_baseline(
    outcome: &FillOutcome,
    history: &FillSummary,
    peers: &[BucketStats],
    today: NaiveDate,
) -> Option<Baseline> {
    if outcome.fill_date.is_some() || outcome.latest_date != today {
        return None;
    }
    let baseline = match history.average_trading_days {
        Some(days) => Baseline::Own(days),
        None => {
            let bucket = YieldBucket::of(outcome.event.cash_dividend, outcome.pre_ex_close)?;
            let median = peers
                .iter()
                .find(|stats| stats.bucket == bucket)?
                .median_trading_days?;
            Baseline::Peer { bucket, median }
        }
    };
    // 基準是 1～120 的交易日數，轉成 u32 不會截斷。
    let first_exceeding = baseline.days().floor() as u32 + 1;
    (outcome.trading_days_since_ex == first_exceeding).then_some(baseline)
}

/// 股利文字：現金與股票股利（元），沒有的那一種不列。
fn dividend_text(cash: Decimal, stock: Decimal) -> String {
    let mut parts = Vec::new();
    if cash > Decimal::ZERO {
        parts.push(format!("現金 {} 元", cash.normalize()));
    }
    if stock > Decimal::ZERO {
        parts.push(format!("股票 {} 元", stock.normalize()));
    }
    parts.join("、")
}

/// 填息目標的說明：沒有配股時就是前一日收盤，有配股時註明由前一日收盤調整而來。
fn target_text(outcome: &FillOutcome) -> String {
    if outcome.fill_target == outcome.pre_ex_close {
        return format!(
            "前一日收盤 {}",
            format_decimal_with_commas(outcome.pre_ex_close)
        );
    }
    format!(
        "填息目標 {}（前一日收盤 {} 依配股調整）",
        format_decimal_with_commas(outcome.fill_target),
        format_decimal_with_commas(outcome.pre_ex_close),
    )
}

/// 組出通知訊息（Telegram MarkdownV2）；沒有填息也沒有落後時回 `None`。
fn build_message(
    date: NaiveDate,
    review: &Review,
    stock_name: impl Fn(&str) -> String,
) -> Option<String> {
    if review.filled.is_empty() && review.lagging.is_empty() {
        return None;
    }
    let mut message =
        String::with_capacity(256 + (review.filled.len() + review.lagging.len()) * 160);
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2(format!("持股填息︰{date}"))
    );
    for (symbol, outcome) in &review.filled {
        let days = outcome.trading_days_to_fill.unwrap_or_default();
        let line = format!(
            "{symbol} {} 除權息 {}（{}）：{}，今日收盤 {}，第 {days} 個交易日填息",
            stock_name(symbol),
            outcome.event.ex_date,
            dividend_text(outcome.event.cash_dividend, outcome.event.stock_dividend),
            target_text(outcome),
            format_decimal_with_commas(outcome.latest_close),
        );
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
    }
    if !review.lagging.is_empty() {
        let _ = writeln!(
            &mut message,
            "\n{}",
            text::escape_markdown_v2("填息落後（今天第一次超過基準）：")
        );
        for lagging in &review.lagging {
            let _ = writeln!(
                &mut message,
                "{}",
                text::escape_markdown_v2(lagging_line(lagging, &stock_name))
            );
        }
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(LAG_FOOTNOTE));
    }
    Some(message)
}

/// 落後段的一行：已過天數、基準、最新收盤與距填息價的差距。
fn lagging_line(lagging: &Lagging, stock_name: impl Fn(&str) -> String) -> String {
    let outcome = &lagging.outcome;
    let gap = outcome
        .gap_percent()
        .map_or_else(|| "—".to_owned(), |gap| format!("{gap}%"));
    format!(
        "{} {} 除權息 {}（{}）：已 {} 個交易日未填息，超過{}；最新收盤 {}，距填息價 {} 還差 {gap}",
        lagging.symbol,
        stock_name(&lagging.symbol),
        outcome.event.ex_date,
        dividend_text(outcome.event.cash_dividend, outcome.event.stock_dividend),
        outcome.trading_days_since_ex,
        lagging.baseline.text(),
        format_decimal_with_commas(outcome.latest_close),
        format_decimal_with_commas(outcome.fill_target),
    )
}

#[cfg(test)]
mod tests;
