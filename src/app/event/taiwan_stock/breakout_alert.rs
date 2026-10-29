//! # 全市場區間突破通知
//!
//! 每個交易日 19:20 讀收盤後預先算好的區間突破訊號（`range_breakout`，見
//! [`crate::domain::quote::technical::breakout`]），依當日成交值列出前 [`LISTED`] 檔：代號名稱、產業與
//! 訊號說明（突破價、整理幅度、量比、距突破價）；持股另外標「持股」。附近一年全市場同一訊號出現後
//! 20 個交易日的勝率與平均還原報酬（樣本至少 [`MIN_OUTCOME_SAMPLES`] 次才顯示）。
//!
//! 排在 19:15 持股訊號通知之後。沒有任何訊號（含休市日）時不發送。只供參考，不構成投資建議。

use std::collections::BTreeSet;
use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Days, Local, NaiveDate};

use super::technical_alert::MIN_OUTCOME_SAMPLES;
use crate::{
    core::{alert, util::text},
    domain::{
        portfolio::repository::PortfolioRepository,
        quote::technical::{
            OUTCOME_HORIZON, RANGE_BREAKOUT_INDICATOR, RankedSignal, RankedSignalQuery,
            SignalOutcome, TechnicalSignalQuery,
        },
    },
    infra::database::repository::{portfolio::PgPortfolioRepository, quote::PgQuoteRepository},
};

/// 最多列出幾檔。
const LISTED: i64 = 15;

/// 歷史表現統計的回看天數（日曆天）。
const OUTCOME_LOOKBACK_DAYS: u64 = 364;

/// 排程入口：讀今天的區間突破訊號並發送通知。
///
/// # Errors
///
/// 查詢訊號失敗時回傳錯誤；持股或歷史表現讀取失敗只記錄警告，照常通知。
pub async fn execute() -> Result<()> {
    let holdings: BTreeSet<String> = match PgPortfolioRepository::new()
        .fetch_active_holdings(None)
        .await
    {
        Ok(holdings) => holdings
            .into_iter()
            .map(|holding| holding.security_code)
            .collect(),
        Err(why) => {
            tracing::warn!("區間突破通知：讀取持股失敗，不標持股: {why:#}");
            BTreeSet::new()
        }
    };
    let date = Local::now().date_naive();
    if let Some(message) = run(&PgQuoteRepository::new(), date, &holdings).await? {
        alert::send_message(&message).await;
    }
    Ok(())
}

/// 讀取當天的區間突破與歷史表現並組出通知；沒有訊號時回 `None`。
async fn run<R>(repo: &R, date: NaiveDate, holdings: &BTreeSet<String>) -> Result<Option<String>>
where
    R: RankedSignalQuery + TechnicalSignalQuery,
{
    let (signals, total) = repo
        .fetch_ranked_signals(date, RANGE_BREAKOUT_INDICATOR, LISTED)
        .await
        .context("fetch range breakout signals failed")?;
    tracing::info!("區間突破通知: total={total} listed={}", signals.len());
    if signals.is_empty() {
        return Ok(None);
    }
    let from = date - Days::new(OUTCOME_LOOKBACK_DAYS);
    let outcome = repo
        .fetch_signal_outcomes(from, date, &[RANGE_BREAKOUT_INDICATOR])
        .await
        .unwrap_or_else(|why| {
            tracing::warn!("區間突破通知：讀取訊號歷史表現失敗，不附勝率: {why:#}");
            Vec::new()
        })
        .into_iter()
        .find(|outcome| outcome.samples >= MIN_OUTCOME_SAMPLES);
    Ok(Some(build_message(
        date,
        &signals,
        total,
        outcome.as_ref(),
        holdings,
    )))
}

/// 組出通知內容（MarkdownV2 已跳脫）。
fn build_message(
    date: NaiveDate,
    signals: &[RankedSignal],
    total: i64,
    outcome: Option<&SignalOutcome>,
    holdings: &BTreeSet<String>,
) -> String {
    let mut message = String::with_capacity(256 + signals.len() * 128);
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2(format!(
            "區間突破︰{date}（全市場 {total} 檔，依成交值列前 {}）",
            signals.len()
        ))
    );
    for signal in signals {
        let mut title = format!("{} {}", signal.stock_symbol, signal.name);
        if !signal.industry.is_empty() {
            let _ = write!(&mut title, "（{}）", signal.industry);
        }
        if holdings.contains(&signal.stock_symbol) {
            title.push_str("【持股】");
        }
        let _ = write!(
            &mut message,
            "\n{}\n{}\n",
            text::escape_markdown_v2(title),
            text::escape_markdown_v2(format!("• {}", signal.description))
        );
    }
    let mut footer = String::new();
    if let Some(outcome) = outcome {
        let _ = writeln!(
            &mut footer,
            "＊近一年全市場區間突破後 {OUTCOME_HORIZON} 個交易日：勝率 {:.0}%、平均還原報酬 {:+.1}%（{} 次），過去表現不代表未來結果",
            outcome.win_rate * 100.0,
            outcome.avg_return,
            outcome.samples
        );
    }
    footer.push_str("＊2019 年起回測：單獨依區間突破選股明顯輸 0050，只供參考，不構成投資建議");
    let _ = write!(&mut message, "\n{}\n", text::escape_markdown_v2(footer));
    message
}

#[cfg(test)]
mod tests;
