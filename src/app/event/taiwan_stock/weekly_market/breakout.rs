//! 每週市場摘要的區間突破一段：近一週每天區間突破的檔次合計、成交值前 [`TOP`] 檔，以及近一年同訊號
//! 出現後 20 個交易日的勝率與平均還原報酬。

use anyhow::Result;
use chrono::{Days, NaiveDate};

use super::{TOP, WEEK_DAYS};
use crate::app::event::taiwan_stock::technical_alert::MIN_OUTCOME_SAMPLES;
use crate::domain::quote::{
    forward_return::ForwardReturnRepository,
    technical::{
        OUTCOME_HORIZON, RANGE_BREAKOUT_INDICATOR, RankedSignal, RankedSignalQuery, SignalKind,
        SignalOutcome, TechnicalSignalQuery,
    },
};

/// 歷史表現統計的回看天數（日曆天）。
const OUTCOME_LOOKBACK_DAYS: u64 = 364;

/// 一週的區間突破。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct BreakoutSection {
    pub(super) from: NaiveDate,
    pub(super) to: NaiveDate,
    /// 一週內每天的訊號檔數合計（同一檔不同天各算一次）。
    pub(super) total: i64,
    /// 一週內成交值最大的前 [`TOP`] 檔（同一檔只取成交值最大的那天）。
    pub(super) top: Vec<RankedSignal>,
    /// 近一年同訊號表現；樣本不足或讀取失敗時為 `None`。
    pub(super) outcome: Option<SignalOutcome>,
}

/// 最近 [`WEEK_DAYS`] 個交易日的區間突破；沒有交易日回 `None`。
pub(super) async fn breakout_section(
    quote: &(impl ForwardReturnRepository + RankedSignalQuery + TechnicalSignalQuery),
    date: NaiveDate,
) -> Result<Option<BreakoutSection>> {
    let dates = quote.fetch_trading_dates(date, WEEK_DAYS).await?;
    let (Some(&to), Some(&from)) = (dates.first(), dates.last()) else {
        return Ok(None);
    };
    let mut total = 0;
    let mut top: Vec<RankedSignal> = Vec::new();
    for day in &dates {
        let (signals, count) = quote
            .fetch_ranked_signals(
                *day,
                RANGE_BREAKOUT_INDICATOR,
                SignalKind::Breakout,
                TOP as i64,
            )
            .await?;
        total += count;
        top.extend(signals);
    }
    top.sort_by(|a, b| {
        b.trade_value
            .total_cmp(&a.trade_value)
            .then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
    });
    let mut seen = std::collections::HashSet::new();
    top.retain(|signal| seen.insert(signal.stock_symbol.clone()));
    top.truncate(TOP);
    let outcome = match quote
        .fetch_signal_outcomes(
            to - Days::new(OUTCOME_LOOKBACK_DAYS),
            to,
            &[RANGE_BREAKOUT_INDICATOR],
        )
        .await
    {
        Ok(outcomes) => outcomes.into_iter().find(|outcome| {
            outcome.kind == SignalKind::Breakout.as_str() && outcome.samples >= MIN_OUTCOME_SAMPLES
        }),
        Err(why) => {
            tracing::warn!("每週市場摘要：讀取區間突破歷史表現失敗，不附勝率: {why:#}");
            None
        }
    };
    Ok(Some(BreakoutSection {
        from,
        to,
        total,
        top,
        outcome,
    }))
}

/// 區間突破一段的各行（開頭為空行）；一週都沒有訊號時回空陣列。
pub(super) fn lines(section: &BreakoutSection) -> Vec<String> {
    if section.total == 0 {
        return Vec::new();
    }
    let examples: Vec<String> = section
        .top
        .iter()
        .map(|signal| format!("{} {}", signal.stock_symbol, signal.name))
        .collect();
    let mut lines = vec![
        String::new(),
        format!("區間突破（{}～{}）", section.from, section.to),
        format!(
            "• 共 {} 檔次；成交值前 {}：{}",
            section.total,
            examples.len(),
            examples.join("、")
        ),
    ];
    if let Some(outcome) = &section.outcome {
        lines.push(format!(
            "• 近一年同訊號 {OUTCOME_HORIZON} 日勝率 {:.0}%、平均還原報酬 {:+.1}%（{} 次）；回測單獨使用輸 0050",
            outcome.win_rate * 100.0,
            outcome.avg_return,
            outcome.samples
        ));
    }
    lines
}
