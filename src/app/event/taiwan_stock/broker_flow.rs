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
//!
//! 抓到的今天資料（不論是否達門檻）都寫入 `broker_flow` 表。
//!
//! 每天只跑一次，不另外記錄已通知；服務在 20:40 之後才重啟也不會補發。

use std::fmt::Write;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::{add_thousand_separators, format_decimal_with_commas, holdings};
use crate::{
    app::backfill,
    core::{alert, util::text},
    infra::crawler::fbs::broker_flow::{self, BrokerFlow, BrokerNet},
};

/// 主力買賣超佔成交量比重（%）的通知門檻。
///
/// 2026-10-05 持股 44 檔普通股中約三分之一超過 20%，再低會讓每天的訊息太長。
pub const MAIN_SHARE_THRESHOLD: Decimal = dec!(20);

/// 主力買賣超張數的通知門檻；成交清淡的股票幾十張就能有很高的比重，不具意義。
pub const MIN_MAIN_NET_LOTS: i64 = 100;

/// 每側列出的分點數。
const TOP_BROKERS: usize = 3;

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
        let message = build_message(today, &notable, summary.fetched, holdings::stock_name);
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

/// 主力動作是否明顯：比重與張數都達門檻。
fn is_notable(flow: &BrokerFlow) -> bool {
    flow.main_share().abs() >= MAIN_SHARE_THRESHOLD && flow.main_net().abs() >= MIN_MAIN_NET_LOTS
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
        let _ = write!(
            &mut message,
            "\n{}\n{}\n{}\n",
            text::escape_markdown_v2(headline),
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
mod tests {
    use super::*;

    fn broker(name: &str, net: i64, share: Decimal) -> BrokerNet {
        BrokerNet {
            name: name.to_string(),
            buy: net,
            sell: 0,
            net,
            share,
        }
    }

    /// 以買賣超側各一家分點組出指定主力買賣超與比重的資料。
    fn flow(stock_symbol: &str, buy: (i64, Decimal), sell: (i64, Decimal)) -> BrokerFlow {
        BrokerFlow {
            stock_symbol: stock_symbol.to_string(),
            date: NaiveDate::from_ymd_opt(2026, 10, 5).unwrap(),
            buyers: vec![broker("買方", buy.0, buy.1)],
            sellers: vec![broker("賣方", sell.0, sell.1)],
            buy_total: buy.0,
            sell_total: sell.0,
        }
    }

    /// 比重與張數都要達門檻；剛好等於門檻也算。
    #[test]
    fn is_notable_requires_both_share_and_lots() {
        assert!(is_notable(&flow("2330", (8_109, dec!(31.2)), (0, dec!(0)))));
        assert!(is_notable(&flow("1101", (0, dec!(0)), (100, dec!(20)))));
        assert!(
            !is_notable(&flow("2887", (6_940, dec!(19.99)), (0, dec!(0)))),
            "比重未達"
        );
        assert!(
            !is_notable(&flow("1232", (99, dec!(60)), (0, dec!(0)))),
            "張數未達"
        );
        assert!(
            !is_notable(&flow("2880", (500, dec!(25)), (500, dec!(25)))),
            "買賣相抵"
        );
    }

    /// 只留達門檻的股票，買超比重大的在前、賣超比重大的在最後。
    #[test]
    fn notable_flows_filters_and_sorts_by_share() {
        let flows = vec![
            flow("1101", (0, dec!(0)), (11_926, dec!(33.37))),
            flow("2353", (1_174, dec!(9.75)), (0, dec!(0))),
            flow("2330", (8_109, dec!(30.72)), (0, dec!(0))),
            flow("1303", (19_579, dec!(21.55)), (0, dec!(0))),
        ];
        let symbols: Vec<String> = notable_flows(flows)
            .into_iter()
            .map(|flow| flow.stock_symbol)
            .collect();
        assert_eq!(symbols, vec!["2330", "1303", "1101"]);
    }

    /// 最多列三家；買賣相抵為 0 的分點不列，整側沒有就寫「無」。
    #[test]
    fn top_brokers_lists_up_to_three_with_volume() {
        let brokers = vec![
            broker("摩根大通", 2_663, dec!(10.25)),
            broker("台灣摩根", 2_324, dec!(8.94)),
            broker("元大", 1_740, dec!(6.7)),
            broker("美商高盛", 1_417, dec!(5.45)),
        ];
        assert_eq!(
            top_brokers(&brokers),
            "摩根大通 2,663、台灣摩根 2,324、元大 1,740"
        );
        assert_eq!(top_brokers(&[broker("元大-三峽", 0, dec!(0))]), "無");
        assert_eq!(top_brokers(&[]), "無");
    }

    /// 訊息含日期、門檻說明與每檔的主力方向、張數、比重及前幾名分點，特殊字元依 MarkdownV2 跳脫。
    #[test]
    fn build_message_lists_each_stock() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let flows = vec![
            flow("2330", (8_109, dec!(30.72)), (0, dec!(0))),
            flow("1101", (0, dec!(0)), (11_926, dec!(33.37))),
        ];
        let message = build_message(date, &flows, 44, |symbol| match symbol {
            "2330" => "台積電".to_string(),
            _ => "台泥".to_string(),
        });

        assert!(message.starts_with("持股主力進出︰2026\\-10\\-05\n"));
        assert!(message.contains("主力買賣超佔成交量達 20% 的持股（今日資料 44 檔）"));
        assert!(
            message
                .contains("\n2330 台積電 主力買超 8,109 張（30\\.72%）\n買︰買方 8,109\n賣︰無\n")
        );
        assert!(
            message
                .contains("\n1101 台泥 主力賣超 11,926 張（33\\.37%）\n買︰無\n賣︰賣方 11,926\n")
        );
    }
}
