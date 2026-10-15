//! # 持股技術訊號通知
//!
//! 每個交易日晚上讀收盤後預先算好的技術訊號（`daily_technical_signal`，見
//! [`crate::app::calculation::technical_signal`]），列出持股出現的重要訊號。
//!
//! - 持股包含 ETF（例如 0050 創新高、0056 跌破年線也值得知道），不含已售出的持股。
//! - 只通知中長期、出現頻率低的訊號（[`NOTIFY_INDICATORS`]）：52 週新高新低、收盤站上或跌破
//!   季線與年線、20 日與 60 日均線交叉。KD、RSI 超買超賣與短天期均線幾乎天天有，不通知。
//! - 每個訊號附上近一年全市場同一訊號出現後 20 個交易日的勝率與平均還原報酬（樣本至少
//!   [`MIN_OUTCOME_SAMPLES`] 次才顯示）；統計查詢失敗只記錄警告，照常通知。
//! - 另列當天相對強度（RS，見 [`crate::domain::quote::relative_strength`]）穿越 80 或 50 分的
//!   持股（與前一個交易日比較）；當天報酬還沒回填就不列，查詢失敗只記錄警告。
//! - 排程在 19:15（週一至週五），排在 19:00 收盤資料完整性檢查之後，重跑收盤匯總的訊號也已定案。
//! - 沒有任何持股出現訊號或 RS 轉折（含休市日）時不發送。

use std::collections::BTreeSet;
use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Days, Local, NaiveDate};

use super::holdings;
use crate::{
    core::{alert, util::text},
    domain::{
        portfolio::repository::PortfolioRepository,
        quote::{
            relative_strength::{
                CrossDirection, RelativeStrengthDay, RelativeStrengthQuery, crossings,
            },
            technical::{
                FIFTY_TWO_WEEK_INDICATOR, OUTCOME_HORIZON, SignalOutcome, StockTechnicalSignal,
                TechnicalSignalQuery,
            },
        },
    },
    infra::database::repository::{portfolio::PgPortfolioRepository, quote::PgQuoteRepository},
};

/// 要通知的訊號來源，也是同一檔股票內訊號的排列順序。
pub const NOTIFY_INDICATORS: [&str; 4] = [
    FIFTY_TWO_WEEK_INDICATOR,
    "close_ma240",
    "close_ma60",
    "ma20_ma60",
];

/// 歷史表現統計的回看天數（日曆天）。
const OUTCOME_LOOKBACK_DAYS: u64 = 364;

/// 歷史表現樣本少於此數時不顯示（避免少數樣本的勝率誤導）。
pub const MIN_OUTCOME_SAMPLES: i64 = 30;

/// 比較 RS 轉折要讀的交易日數（當天與前一個交易日）。
const RS_DAYS: u16 = 2;

/// 持股當天的相對強度轉折。
#[derive(Debug, Clone, PartialEq)]
struct RsTurn {
    stock_symbol: String,
    /// 前一個交易日的分數。
    previous: u8,
    /// 當天的分數。
    current: u8,
    /// 當天在全體中的名次。
    rank: i64,
    /// 當天計分的股票總數。
    universe: i64,
    direction: CrossDirection,
    /// 穿越的門檻，高門檻在前。
    thresholds: Vec<u8>,
}

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

/// 讀取持股訊號與 RS 轉折並組出通知內容；兩者都沒有時回 `None`。
async fn run(
    repo: &(impl TechnicalSignalQuery + RelativeStrengthQuery),
    date: NaiveDate,
    symbols: &BTreeSet<String>,
) -> Result<Option<String>> {
    let symbols: Vec<String> = symbols.iter().cloned().collect();
    let signals = repo
        .fetch_signals(date, &symbols)
        .await
        .context("fetch technical signals of holdings failed")?;
    let notable = notable(signals);
    let turns = match repo.fetch_relative_strength(date, RS_DAYS, &symbols).await {
        Ok(days) => rs_turns(&days, date),
        Err(why) => {
            tracing::warn!("持股技術訊號通知：讀取相對強度失敗，不列 RS 轉折: {why:#}");
            Vec::new()
        }
    };
    tracing::info!(
        "持股技術訊號通知: holdings={} signals={} rs_turns={}",
        symbols.len(),
        notable.len(),
        turns.len()
    );
    if notable.is_empty() && turns.is_empty() {
        return Ok(None);
    }
    let outcomes = if notable.is_empty() {
        Vec::new()
    } else {
        let from = date - Days::new(OUTCOME_LOOKBACK_DAYS);
        repo.fetch_signal_outcomes(from, date, &NOTIFY_INDICATORS)
            .await
            .unwrap_or_else(|why| {
                tracing::warn!("持股技術訊號通知：讀取訊號歷史表現失敗，不附勝率: {why:#}");
                Vec::new()
            })
    };
    Ok(build_message(
        date,
        &notable,
        &outcomes,
        &turns,
        holdings::stock_name,
    ))
}

/// 由依代號、再由新到舊排列的分數找出當天穿越門檻的持股；最新一筆不是 `date`（當天報酬
/// 還沒回填）或沒有前一個交易日的分數就略過。
fn rs_turns(days: &[RelativeStrengthDay], date: NaiveDate) -> Vec<RsTurn> {
    days.chunk_by(|a, b| a.stock_symbol == b.stock_symbol)
        .filter_map(|group| {
            let [latest, previous, ..] = group else {
                return None;
            };
            if latest.date != date {
                return None;
            }
            let (current, before) = (latest.rs_score, previous.rs_score);
            let crossed = crossings(before, current);
            let direction = crossed.first()?.1;
            Some(RsTurn {
                stock_symbol: latest.stock_symbol.clone(),
                previous: before,
                current,
                rank: latest.rank,
                universe: latest.universe,
                direction,
                thresholds: crossed
                    .into_iter()
                    .map(|(threshold, _)| threshold)
                    .collect(),
            })
        })
        .collect()
}

/// RS 轉折的說明，例如「相對強度 80 → 79，跌破 80 分（全體第 446 名／2228 檔）」。
fn turn_text(turn: &RsTurn) -> String {
    let verb = match turn.direction {
        CrossDirection::Up => "突破",
        CrossDirection::Down => "跌破",
    };
    let thresholds: Vec<String> = turn.thresholds.iter().map(u8::to_string).collect();
    format!(
        "相對強度 {} → {}，{verb} {} 分（全體第 {} 名／{} 檔）",
        turn.previous,
        turn.current,
        thresholds.join("、"),
        turn.rank,
        turn.universe
    )
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

/// 依股票（代號排序）分段列出訊號說明（附同一訊號的歷史表現）與 RS 轉折；兩者都沒有時回 `None`。
fn build_message(
    date: NaiveDate,
    signals: &[StockTechnicalSignal],
    outcomes: &[SignalOutcome],
    turns: &[RsTurn],
    stock_name: impl Fn(&str) -> String,
) -> Option<String> {
    if signals.is_empty() && turns.is_empty() {
        return None;
    }
    let stocks: BTreeSet<&str> = signals
        .iter()
        .map(|signal| signal.stock_symbol.as_str())
        .chain(turns.iter().map(|turn| turn.stock_symbol.as_str()))
        .collect();
    let mut message = String::with_capacity(128 + (signals.len() + turns.len()) * 96);
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
        for turn in turns.iter().filter(|turn| turn.stock_symbol == symbol) {
            let _ = writeln!(
                &mut message,
                "{}",
                text::escape_markdown_v2(format!("• {}", turn_text(turn)))
            );
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
