//! # 持股技術訊號通知
//!
//! 每個交易日晚上讀收盤後預先算好的技術訊號（`daily_technical_signal`，見
//! [`crate::app::calculation::technical_signal`]），列出持股出現的重要訊號。
//!
//! - 持股包含 ETF（例如 0050 創新高、0056 跌破年線也值得知道），不含已售出的持股。
//! - 只通知中長期、出現頻率低的訊號（[`NOTIFY_INDICATORS`]）：52 週新高新低、收盤站上或跌破
//!   季線與年線、20 日與 60 日均線交叉，以及相對強度（RS，見
//!   [`crate::domain::quote::relative_strength`]）比前一個交易日站上或跌破 80、50 分。
//!   KD、RSI 超買超賣與短天期均線幾乎天天有，不通知。
//! - 每個訊號附上近一年全市場同一訊號出現後 20 個交易日的勝率與平均還原報酬（樣本至少
//!   [`MIN_OUTCOME_SAMPLES`] 次才顯示）；統計查詢失敗只記錄警告，照常通知。
//! - 排程在 19:15（週一至週五），排在 19:00 收盤資料完整性檢查之後，重跑收盤匯總的訊號也已定案。
//! - 沒有任何持股出現訊號（含休市日）時不發送。

use std::collections::BTreeSet;
use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Days, Local, NaiveDate};

use super::holdings;
use crate::{
    core::{alert, util::text},
    domain::{
        portfolio::repository::PortfolioRepository,
        quote::technical::{
            FIFTY_TWO_WEEK_INDICATOR, OUTCOME_HORIZON, RS50_INDICATOR, RS80_INDICATOR,
            SignalOutcome, StockTechnicalSignal, TechnicalSignalQuery,
        },
    },
    infra::database::repository::{portfolio::PgPortfolioRepository, quote::PgQuoteRepository},
};

/// 要通知的訊號來源，也是同一檔股票內訊號的排列順序。
pub const NOTIFY_INDICATORS: [&str; 6] = [
    FIFTY_TWO_WEEK_INDICATOR,
    "close_ma240",
    "close_ma60",
    "ma20_ma60",
    RS80_INDICATOR,
    RS50_INDICATOR,
];

/// 歷史表現統計的回看天數（日曆天）。
const OUTCOME_LOOKBACK_DAYS: u64 = 364;

/// 歷史表現樣本少於此數時不顯示（避免少數樣本的勝率誤導）。
pub const MIN_OUTCOME_SAMPLES: i64 = 30;

/// 排程入口：讀今天持股的技術訊號並發送通知。
///
/// # Errors
///
/// 查詢持股或訊號失敗時回傳錯誤。
pub async fn execute() -> Result<()> {
    let holdings = PgPortfolioRepository::new()
        .fetch_active_holdings(None)
        .await
        .context("fetch active holdings for technical alert failed")?;
    let symbols: BTreeSet<String> = holdings
        .into_iter()
        .map(|holding| holding.security_code)
        .collect();
    let date = Local::now().date_naive();
    let message = run(&PgQuoteRepository::new(), date, &symbols).await?;
    if let Some(message) = message {
        alert::send_message(&message).await;
    }
    Ok(())
}

/// 讀取持股訊號並組出通知內容；沒有要通知的訊號時回 `None`。
async fn run(
    repo: &impl TechnicalSignalQuery,
    date: NaiveDate,
    symbols: &BTreeSet<String>,
) -> Result<Option<String>> {
    let symbols: Vec<String> = symbols.iter().cloned().collect();
    let signals = repo
        .fetch_signals(date, &symbols)
        .await
        .context("fetch technical signals of holdings failed")?;
    let notable = notable(signals);
    tracing::info!(
        "持股技術訊號通知: holdings={} signals={}",
        symbols.len(),
        notable.len()
    );
    if notable.is_empty() {
        return Ok(None);
    }
    let from = date - Days::new(OUTCOME_LOOKBACK_DAYS);
    let outcomes = repo
        .fetch_signal_outcomes(from, date, &NOTIFY_INDICATORS)
        .await
        .unwrap_or_else(|why| {
            tracing::warn!("持股技術訊號通知：讀取訊號歷史表現失敗，不附勝率: {why:#}");
            Vec::new()
        });
    Ok(build_message(
        date,
        &notable,
        &outcomes,
        holdings::stock_name,
    ))
}

/// 只留下 [`NOTIFY_INDICATORS`] 的訊號，依代號、再依來源的通知順序排列。
fn notable(mut signals: Vec<StockTechnicalSignal>) -> Vec<StockTechnicalSignal> {
    signals.retain(|signal| NOTIFY_INDICATORS.contains(&signal.signal.indicator));
    signals.sort_by_key(|signal| {
        (
            signal.stock_symbol.clone(),
            NOTIFY_INDICATORS
                .iter()
                .position(|indicator| *indicator == signal.signal.indicator),
        )
    });
    signals
}

/// 依股票（代號排序）分段列出訊號說明（附同一訊號的歷史表現）；沒有訊號時回 `None`。
fn build_message(
    date: NaiveDate,
    signals: &[StockTechnicalSignal],
    outcomes: &[SignalOutcome],
    stock_name: impl Fn(&str) -> String,
) -> Option<String> {
    if signals.is_empty() {
        return None;
    }
    let stocks: BTreeSet<&str> = signals
        .iter()
        .map(|signal| signal.stock_symbol.as_str())
        .collect();
    let mut message = String::with_capacity(128 + signals.len() * 96);
    let _ = writeln!(
        &mut message,
        "持股技術訊號︰{}",
        text::escape_markdown_v2(date.to_string())
    );
    let mut with_outcome = false;
    for symbol in stocks {
        let _ = write!(
            &mut message,
            "\n{}\n",
            text::escape_markdown_v2(format!("{symbol} {}", stock_name(symbol)))
        );
        for signal in signals
            .iter()
            .filter(|signal| signal.stock_symbol == symbol)
        {
            let (line, attached) = signal_line(signal, outcomes);
            with_outcome |= attached;
            let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
        }
    }
    if with_outcome {
        let _ = write!(
            &mut message,
            "\n{}\n",
            text::escape_markdown_v2(format!(
                "＊括號內為近一年全市場同一訊號出現後 {OUTCOME_HORIZON} 個交易日的還原報酬，過去表現不代表未來結果"
            ))
        );
    }
    Some(message)
}

/// 訊號的一行說明，樣本足夠時附上歷史表現；第二個值表示是否附了歷史表現。
fn signal_line(signal: &StockTechnicalSignal, outcomes: &[SignalOutcome]) -> (String, bool) {
    let outcome = outcomes.iter().find(|outcome| {
        outcome.indicator == signal.signal.indicator
            && outcome.kind == signal.signal.kind.as_str()
            && outcome.samples >= MIN_OUTCOME_SAMPLES
    });
    match outcome {
        Some(outcome) => (
            format!(
                "• {}（{}）",
                signal.signal.description,
                outcome_text(outcome)
            ),
            true,
        ),
        None => (format!("• {}", signal.signal.description), false),
    }
}

/// 歷史表現的簡短文字，例如「20 日勝率 58%、平均 +1.2%，1234 次」。
fn outcome_text(outcome: &SignalOutcome) -> String {
    format!(
        "{OUTCOME_HORIZON} 日勝率 {:.0}%、平均 {:+.1}%，{} 次",
        outcome.win_rate * 100.0,
        outcome.avg_return,
        outcome.samples
    )
}

#[cfg(test)]
mod tests;
