//! # 持股主力進出通知
//!
//! 每個交易日盤後檢查持股普通股的主力進出（買超、賣超前 15 名券商分點），
//! 把主力動作明顯的股票彙整成一則 Telegram，列出買超、賣超前 [`TOP_BROKERS`] 名分點。
//!
//! ## 流程（週一至週五 20:40 排程）
//!
//! 1. 取目前持股的普通股代號（[`holdings::common_stock_holdings`]）。
//! 2. 逐檔抓富邦證券（MoneyDJ）主力進出頁；頁面日期不是今天（休市或尚未更新）的股票略過。
//! 3. 主力買賣超佔成交量達 [`MAIN_SHARE_THRESHOLD`]%、且張數達 [`MIN_MAIN_NET_LOTS`] 張的股票
//!    才放進通知，依佔成交量比重由買超到賣超排列；沒有任何一檔達標就不發訊息。
//! 4. 每檔另附含今天在內最近 [`HISTORY_DAYS`] 個有主力資料的交易日的主力累計，看今天是延續還是
//!    反轉；讀取失敗或只有今天一天時不附。
//!
//! 抓到的今天資料（不論是否達門檻）都寫入 `broker_flow` 表。
//!
//! 每天只跑一次，不另外記錄已通知；服務在 20:40 之後才重啟也不會補發。

use std::collections::HashMap;
use std::fmt::Write;
use std::time::Duration;

use super::{add_thousand_separators, format_decimal_with_commas, holdings};
use crate::{
    app::backfill,
    core::{alert, util::text},
    domain::portfolio::chip::{self, BrokerPeriodNet, HoldingBrokerQuery},
    infra::{
        crawler::fbs::broker_flow::{self, BrokerFlow, BrokerNet},
        database::repository::portfolio::PgPortfolioRepository,
    },
};
use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};

// 門檻與 `/portfolio/chip`、stock_go 持股籌碼頁共用，定義在 domain。
pub use crate::domain::portfolio::chip::{
    BROKER_MIN_LOTS as MIN_MAIN_NET_LOTS, BROKER_MIN_SHARE as MAIN_SHARE_THRESHOLD,
};

/// 每側列出的分點數。
const TOP_BROKERS: usize = 3;

/// 主力累計涵蓋的交易日數（含今天）。
const HISTORY_DAYS: usize = 5;

/// 股票之間的請求間隔。
const STOCK_INTERVAL: Duration = Duration::from_millis(500);

/// 一輪通知的結果。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RunSummary {
    /// 檢查的持股檔數。
    pub holdings: usize,
    /// 取得今天資料的檔數。
    pub fetched: usize,
    /// 頁面還是舊日期（休市或尚未更新）的檔數。
    pub stale: usize,
    /// 抓取失敗的檔數。
    pub failed: usize,
    /// 達門檻、放進通知的檔數。
    pub notable: usize,
}

/// 排程入口：通知持股中主力買賣超明顯的股票。
///
/// 單檔失敗只記錄、不中斷；只有查不到持股時回傳錯誤。
pub async fn execute() -> Result<()> {
    let symbols = holdings::common_stock_holdings()
        .await
        .context("fetch active holdings for broker flow notification failed")?;

    let today = Local::now().date_naive();
    let mut summary = RunSummary {
        holdings: symbols.len(),
        ..Default::default()
    };
    let mut flows = Vec::with_capacity(symbols.len());
    for symbol in &symbols {
        match broker_flow::visit(symbol).await {
            Ok(flow) if flow.date == today => flows.push(flow),
            Ok(_) => summary.stale += 1,
            Err(why) => {
                summary.failed += 1;
                tracing::warn!("主力進出抓取失敗: stock_symbol={symbol}, error={why:#}");
            }
        }
        tokio::time::sleep(STOCK_INTERVAL).await;
    }

    summary.fetched = flows.len();
    backfill::chip::save_broker_flows(&flows).await;
    let notable = notable_flows(flows);
    summary.notable = notable.len();
    if !notable.is_empty() {
        let symbols: Vec<String> = notable
            .iter()
            .map(|flow| flow.stock_symbol.clone())
            .collect();
        let history = load_history(&PgPortfolioRepository::new(), today, &symbols).await;
        let message = build_message(
            today,
            &notable,
            summary.fetched,
            history.as_ref(),
            holdings::stock_name,
        );
        alert::send_message(&message).await;
    }

    tracing::info!(
        "持股主力進出通知結束: holdings={}, fetched={}, stale={}, failed={}, notable={}",
        summary.holdings,
        summary.fetched,
        summary.stale,
        summary.failed,
        summary.notable
    );
    Ok(())
}

/// 近幾個交易日的主力累計：實際涵蓋的交易日數與各股合計。
#[derive(Debug, Default)]
struct BrokerHistory {
    days: usize,
    nets: HashMap<String, BrokerPeriodNet>,
}

/// 讀取 `today`（含）之前最近 [`HISTORY_DAYS`] 個有主力資料的交易日裡 `symbols` 的主力合計；
/// 只有今天一天時回 `None`，查詢失敗只記錄警告、也回 `None`。
async fn load_history(
    repository: &impl HoldingBrokerQuery,
    today: NaiveDate,
    symbols: &[String],
) -> Option<BrokerHistory> {
    let history = async {
        let dates = repository.fetch_broker_dates(today, HISTORY_DAYS).await?;
        let Some(&from) = dates.last().filter(|_| dates.len() > 1) else {
            return Ok(None);
        };
        let nets = repository.fetch_broker_period(from, today, symbols).await?;
        Ok::<_, anyhow::Error>(Some(BrokerHistory {
            days: dates.len(),
            nets: nets
                .into_iter()
                .map(|net| (net.stock_symbol.clone(), net))
                .collect(),
        }))
    }
    .await;
    history.unwrap_or_else(|why| {
        tracing::warn!("讀取持股主力近日累計失敗: {why:#}");
        None
    })
}

/// 主力累計一行：方向、張數與買超、賣超天數。
fn history_line(days: usize, net: &BrokerPeriodNet) -> String {
    let lots = add_thousand_separators(&net.main_net.abs().to_string());
    let amount = match net.main_net.signum() {
        1 => format!("買超 {lots} 張"),
        -1 => format!("賣超 {lots} 張"),
        _ => "持平".to_string(),
    };
    format!(
        "近 {days} 日主力{amount}（買超 {} 日、賣超 {} 日）",
        net.buy_days, net.sell_days
    )
}

/// 主力動作是否明顯：比重與張數都達門檻。
fn is_notable(flow: &BrokerFlow) -> bool {
    chip::broker_notable(flow.main_share(), flow.main_net())
}

/// 挑出達門檻的股票，依主力佔成交量比重由買超到賣超排列。
fn notable_flows(flows: Vec<BrokerFlow>) -> Vec<BrokerFlow> {
    let mut notable: Vec<BrokerFlow> = flows.into_iter().filter(is_notable).collect();
    notable.sort_by_key(|flow| std::cmp::Reverse(flow.main_share()));
    notable
}

/// 組出通知訊息（Telegram MarkdownV2）。
///
/// `stock_name` 由呼叫端提供（正式流程讀股票主檔快取），測試時可換成固定對照。
fn build_message(
    date: NaiveDate,
    flows: &[BrokerFlow],
    checked: usize,
    history: Option<&BrokerHistory>,
    stock_name: impl Fn(&str) -> String,
) -> String {
    let mut message = String::with_capacity(256 + flows.len() * 256);
    let _ = writeln!(
        &mut message,
        "持股主力進出︰{}",
        text::escape_markdown_v2(date.to_string())
    );
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2(format!(
            "主力買賣超佔成交量達 {MAIN_SHARE_THRESHOLD}% 的持股（今日資料 {checked} 檔）"
        ))
    );
    for flow in flows {
        let net = flow.main_net();
        let direction = if net >= 0 { "買超" } else { "賣超" };
        let headline = format!(
            "{} {} 主力{direction} {} 張（{}%）",
            flow.stock_symbol,
            stock_name(&flow.stock_symbol),
            add_thousand_separators(&net.abs().to_string()),
            format_decimal_with_commas(flow.main_share().abs())
        );
        let _ = write!(&mut message, "\n{}\n", text::escape_markdown_v2(headline));
        if let Some(history) = history
            && let Some(net) = history.nets.get(&flow.stock_symbol)
        {
            let _ = writeln!(
                &mut message,
                "{}",
                text::escape_markdown_v2(history_line(history.days, net))
            );
        }
        let _ = write!(
            &mut message,
            "{}\n{}\n",
            text::escape_markdown_v2(format!("買︰{}", top_brokers(&flow.buyers))),
            text::escape_markdown_v2(format!("賣︰{}", top_brokers(&flow.sellers)))
        );
    }
    message
}

/// 列出前 [`TOP_BROKERS`] 名分點與買賣超張數；買賣相抵為 0 的分點不列。
fn top_brokers(brokers: &[BrokerNet]) -> String {
    let listed: Vec<String> = brokers
        .iter()
        .filter(|broker| broker.net > 0)
        .take(TOP_BROKERS)
        .map(|broker| {
            format!(
                "{} {}",
                broker.name,
                add_thousand_separators(&broker.net.to_string())
            )
        })
        .collect();
    if listed.is_empty() {
        "無".to_string()
    } else {
        listed.join("、")
    }
}

#[cfg(test)]
mod tests;
