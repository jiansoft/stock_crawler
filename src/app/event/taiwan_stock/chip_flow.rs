//! # 持股三大法人與融資融券通知
//!
//! 每個交易日晚上彙整持股普通股的三大法人買賣超與融資融券變化，
//! 只列出有明顯動作的股票，和盤後的主力進出通知（券商分點）互補。
//!
//! ## 流程（週一至週五 21:40 排程）
//!
//! 1. 取目前持股的普通股代號（[`holdings::common_stock_holdings`]）。
//! 2. 抓上市、上櫃的三大法人買賣超與融資融券餘額（官方全市場資料，各一次請求）；
//!    任一來源失敗只記錄，其餘照常。全部都沒有資料（休市或尚未公布）就結束。
//! 3. 更新 Redis 中每檔外資、投信的連續買賣超天數（`chip:streak:{代號}`），同一天重跑不重複累加。
//! 4. 符合任一條件才列入通知：
//!    - 外資買賣超達當日成交量 [`FOREIGN_MIN_PERCENT`]% 且 [`FOREIGN_MIN_LOTS`] 張以上
//!    - 投信買賣超達當日成交量 [`TRUST_MIN_PERCENT`]% 且 [`TRUST_MIN_LOTS`] 張以上
//!    - 外資連續 [`FOREIGN_STREAK_MIN`] 天、投信連續 [`TRUST_STREAK_MIN`] 天同向買賣超
//!    - 融資餘額增減達 [`MARGIN_MIN_PERCENT`]% 且 [`MARGIN_MIN_LOTS`] 張以上
//!    - 融券餘額增減達 [`SHORT_MIN_PERCENT`]% 且 [`SHORT_MIN_LOTS`] 張以上

use std::collections::HashMap;
use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

use super::{add_thousand_separators, holdings};
use crate::{
    core::{alert, util::text},
    domain::quote::repository::QuoteRepository,
    infra::{
        crawler::{
            share::{InstitutionalFlow, MarginBalance},
            tpex, twse,
        },
        database::repository::quote::PgQuoteRepository,
        nosql::redis::{CLIENT, RedisError},
    },
};

// 門檻與 `/portfolio/chip`、stock_go 持股籌碼頁共用，定義在 domain。
pub use crate::domain::portfolio::chip::{
    FOREIGN_MIN_LOTS, FOREIGN_MIN_PERCENT, FOREIGN_STREAK_MIN, MARGIN_MIN_LOTS, MARGIN_MIN_PERCENT,
    SHORT_MIN_LOTS, SHORT_MIN_PERCENT, TRUST_MIN_LOTS, TRUST_MIN_PERCENT, TRUST_STREAK_MIN,
};
use crate::domain::portfolio::chip::{balance_changed as changed, heavy_flow};

/// 連續天數紀錄的保存時間（30 天）；中斷超過就重新起算。
const STREAK_TTL_SECONDS: usize = 60 * 60 * 24 * 30;

/// 外資、投信連續同向買賣超天數；正值為連買、負值為連賣、0 為當天沒有買賣超。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Streak {
    /// 最後一次更新的交易日。
    date: NaiveDate,
    foreign: i32,
    trust: i32,
}

/// 單一持股當天的籌碼摘要（只有符合條件的股票才會產生）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChipReport {
    stock_symbol: String,
    flow: Option<InstitutionalFlow>,
    margin: Option<MarginBalance>,
    /// 當日成交量（股）。
    volume: Option<i64>,
    streak: Streak,
}

/// 一輪通知的結果。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RunSummary {
    pub holdings: usize,
    pub with_flow: usize,
    pub with_margin: usize,
    pub reported: usize,
}

/// 排程入口：通知持股中三大法人或融資融券有明顯動作的股票。
///
/// 來源或單檔失敗只記錄、不中斷；只有查不到持股時回傳錯誤。
pub async fn execute() -> Result<()> {
    let symbols = holdings::common_stock_holdings()
        .await
        .context("fetch active holdings for chip flow notification failed")?;
    let date = Local::now().date_naive();

    let (listed_flows, otc_flows, listed_margins, otc_margins) = tokio::join!(
        twse::chip::visit_institutional(date),
        tpex::chip::visit_institutional(date),
        twse::chip::visit_margin(date),
        tpex::chip::visit_margin(date),
    );
    let flows = merge("三大法人", [("上市", listed_flows), ("上櫃", otc_flows)]);
    let margins = merge(
        "融資融券",
        [("上市", listed_margins), ("上櫃", otc_margins)],
    );
    if flows.is_empty() && margins.is_empty() {
        tracing::info!("{date} 尚無三大法人與融資融券資料（休市或尚未公布），略過持股籌碼通知");
        return Ok(());
    }

    let volumes: HashMap<String, i64> =
        match PgQuoteRepository::new().fetch_quotes_by_date(date).await {
            Ok(quotes) => quotes
                .into_iter()
                .filter_map(|quote| Some((quote.stock_symbol, quote.trading_volume.to_i64()?)))
                .collect(),
            Err(why) => {
                tracing::warn!("讀取 {date} 成交量失敗，比例條件改為不成立: {why:#}");
                HashMap::new()
            }
        };

    let mut summary = RunSummary {
        holdings: symbols.len(),
        ..Default::default()
    };
    let mut reports = Vec::new();
    for symbol in &symbols {
        let flow = flows.get(symbol).copied();
        let margin = margins.get(symbol).copied();
        summary.with_flow += usize::from(flow.is_some());
        summary.with_margin += usize::from(margin.is_some());

        let previous = load_streak(symbol).await;
        let streak = next_streak(previous, date, flow);
        if previous != Some(streak) {
            save_streak(symbol, &streak).await;
        }
        let report = ChipReport {
            stock_symbol: symbol.clone(),
            flow,
            margin,
            volume: volumes.get(symbol).copied(),
            streak,
        };
        if !reasons(&report).is_empty() {
            reports.push(report);
        }
    }

    summary.reported = reports.len();
    if !reports.is_empty() {
        alert::send_message(&build_message(date, &reports, holdings::stock_name)).await;
    }
    tracing::info!(
        "持股籌碼通知結束: holdings={}, with_flow={}, with_margin={}, reported={}",
        summary.holdings,
        summary.with_flow,
        summary.with_margin,
        summary.reported
    );
    Ok(())
}

/// 合併上市、上櫃的結果；失敗的來源只記錄。
fn merge<T>(kind: &str, sources: [(&str, Result<HashMap<String, T>>); 2]) -> HashMap<String, T> {
    let mut merged = HashMap::new();
    for (market, result) in sources {
        match result {
            Ok(rows) => merged.extend(rows),
            Err(why) => tracing::warn!("{market}{kind}抓取失敗: {why:#}"),
        }
    }
    merged
}

/// 依當天買賣超方向更新連續天數；同一天重跑時維持原值。
fn next_streak(
    previous: Option<Streak>,
    date: NaiveDate,
    flow: Option<InstitutionalFlow>,
) -> Streak {
    let previous = previous.unwrap_or_default();
    if previous.date == date {
        return previous;
    }
    let step = |count: i32, net: Option<i64>| match net.unwrap_or(0).signum() {
        1 if count > 0 => count + 1,
        1 => 1,
        -1 if count < 0 => count - 1,
        -1 => -1,
        _ => 0,
    };
    Streak {
        date,
        foreign: step(previous.foreign, flow.map(|flow| flow.foreign)),
        trust: step(previous.trust, flow.map(|flow| flow.trust)),
    }
}

/// 列出這檔符合的條件；空的代表不需通知。
fn reasons(report: &ChipReport) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    let heavy = |net: i64, min_percent: i64, min_lots: i64| {
        heavy_flow(net, report.volume, min_percent, min_lots)
    };
    if let Some(flow) = report.flow {
        if heavy(flow.foreign, FOREIGN_MIN_PERCENT, FOREIGN_MIN_LOTS) {
            reasons.push("外資大量");
        }
        if heavy(flow.trust, TRUST_MIN_PERCENT, TRUST_MIN_LOTS) {
            reasons.push("投信大量");
        }
    }
    if report.streak.foreign.abs() >= FOREIGN_STREAK_MIN {
        reasons.push("外資連續");
    }
    if report.streak.trust.abs() >= TRUST_STREAK_MIN {
        reasons.push("投信連續");
    }
    if let Some(margin) = report.margin {
        if changed(
            margin.margin_previous,
            margin.margin_today,
            MARGIN_MIN_PERCENT,
            MARGIN_MIN_LOTS,
        ) {
            reasons.push("融資");
        }
        if changed(
            margin.short_previous,
            margin.short_today,
            SHORT_MIN_PERCENT,
            SHORT_MIN_LOTS,
        ) {
            reasons.push("融券");
        }
    }
    reasons
}

/// 組出通知訊息（Telegram MarkdownV2）。
///
/// `stock_name` 由呼叫端提供（正式流程讀股票主檔快取），測試時可換成固定對照。
fn build_message(
    date: NaiveDate,
    reports: &[ChipReport],
    stock_name: impl Fn(&str) -> String,
) -> String {
    let mut message = String::with_capacity(256 + reports.len() * 256);
    let _ = writeln!(
        &mut message,
        "持股法人與信用交易︰{}",
        text::escape_markdown_v2(date.to_string())
    );
    for report in reports {
        let _ = write!(
            &mut message,
            "\n{}\n",
            text::escape_markdown_v2(format!(
                "{} {}",
                report.stock_symbol,
                stock_name(&report.stock_symbol)
            ))
        );
        if let Some(flow) = report.flow {
            let _ = writeln!(
                &mut message,
                "{}",
                text::escape_markdown_v2(format!(
                    "法人︰外資 {}{} 投信 {}{} 自營 {}",
                    signed_lots(flow.foreign),
                    flow_note(flow.foreign, report.volume, report.streak.foreign),
                    signed_lots(flow.trust),
                    flow_note(flow.trust, report.volume, report.streak.trust),
                    signed_lots(flow.dealer)
                ))
            );
        }
        if let Some(margin) = report.margin {
            let _ = writeln!(
                &mut message,
                "{}",
                text::escape_markdown_v2(format!(
                    "信用︰融資 {} 張{} 融券 {} 張{}",
                    add_thousand_separators(&margin.margin_today.to_string()),
                    balance_change(margin.margin_previous, margin.margin_today),
                    add_thousand_separators(&margin.short_today.to_string()),
                    balance_change(margin.short_previous, margin.short_today)
                ))
            );
        }
    }
    message
}

/// 股數轉成帶正負號的張數（四捨五入）。
fn signed_lots(shares: i64) -> String {
    let lots = (shares + shares.signum() * 500) / 1_000;
    let sign = if lots > 0 { "+" } else { "" };
    format!("{sign}{}", add_thousand_separators(&lots.to_string()))
}

/// 買賣超的補充說明：佔成交量比例與連續天數，例如「（11.2%，連 3 買）」。
fn flow_note(net: i64, volume: Option<i64>, streak: i32) -> String {
    let mut parts = Vec::new();
    if let Some(volume) = volume.filter(|volume| *volume > 0 && net != 0) {
        let percent = net.abs() as f64 * 100.0 / volume as f64;
        parts.push(format!("{percent:.1}%"));
    }
    if streak.abs() >= 2 {
        let side = if streak > 0 { "買" } else { "賣" };
        parts.push(format!("連 {} {side}", streak.abs()));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("（{}）", parts.join("，"))
    }
}

/// 餘額增減比例，例如「（−2.6%）」；前日為 0 或沒有變化時不顯示。
fn balance_change(previous: i64, today: i64) -> String {
    if previous == 0 || previous == today {
        return String::new();
    }
    let percent = (today - previous) as f64 * 100.0 / previous as f64;
    format!("（{percent:+.1}%）")
}

/// 連續天數紀錄的 Redis key。
fn streak_key(stock_symbol: &str) -> String {
    format!("chip:streak:{stock_symbol}")
}

/// 讀取連續天數；不存在或讀取失敗時回傳 `None`（重新起算）。
async fn load_streak(stock_symbol: &str) -> Option<Streak> {
    match CLIENT.get_bytes(&streak_key(stock_symbol)).await {
        Ok(bytes) => serde_json::from_slice(&bytes).ok(),
        Err(RedisError::NotFound) => None,
        Err(why) => {
            tracing::warn!("讀取 {stock_symbol} 連續買賣超天數失敗，重新起算: {why}");
            None
        }
    }
}

/// 寫回連續天數；失敗只記錄。
async fn save_streak(stock_symbol: &str, streak: &Streak) {
    let Ok(json) = serde_json::to_string(streak) else {
        return;
    };
    if let Err(why) = CLIENT
        .set(&streak_key(stock_symbol), json, STREAK_TTL_SECONDS)
        .await
    {
        tracing::warn!("寫入 {stock_symbol} 連續買賣超天數失敗: {why:?}");
    }
}

#[cfg(test)]
mod tests;
