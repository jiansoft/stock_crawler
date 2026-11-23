//! # 每週市場摘要
//!
//! 每週六整理最近 [`WEEK_DAYS`] 個交易日的市場輪動，發一則 Telegram：
//!
//! - 大盤趨勢：加權指數收盤、近 [`WEEK_DAYS`] 日漲跌、均線排列、相對 60／240 日均線的位置與方向
//!   （見 [`crate::domain::market_index::regime`]）、這週穿越 20／60／240 日均線的日子，以及市場廣度
//!   （站上 20／60／240 日均線的股票比例、與一週前比較，與這週創 52 週新高／新低的家次，見
//!   [`crate::domain::market_index::breadth`]），以及上市櫃本益比、股價淨值比中位數在近 5 年的位置（見
//!   [`crate::domain::market_index::valuation`]），以及資料日所在月份在前 8 年同月的大盤等權 20 日平均報酬
//!   與上漲比例（見 [`market::month_seasonality`]）。
//! - 產業相對強度：成分股 RS 中位數（見 [`crate::domain::quote::relative_strength`]）最強、最弱，
//!   以及與 [`WEEK_DAYS`] 個交易日前相比轉強、轉弱最多的產業；成分股少於
//!   [`industry::MIN_INDUSTRY_STOCKS`] 檔的產業不列。
//! - 近 [`WEEK_DAYS`] 個交易日相對強度站上、跌破 80 與 50 分的家數（每檔每個門檻只取最後一次
//!   穿越），各附分數最高（站上）或最低（跌破）的幾檔。
//! - 近 [`WEEK_DAYS`] 個交易日的區間突破檔次合計、成交值最大的幾檔與近一年同訊號表現（見 [`breakout`]）。
//! - 外資、投信近 [`WEEK_DAYS`] 個交易日在各產業的估計淨買賣金額（見 [`crate::domain::chip::flow`]），
//!   淨流入與淨流出最多的產業。
//! - 持股（普通股）的 RS 與 [`WEEK_DAYS`] 個交易日前相比上升、下降最多的幾檔。
//! - 外資、投信近 [`WEEK_DAYS`] 個交易日在持股的累計買賣超，買超、賣超最多的幾檔（沿用產業資金一段
//!   已載入的個股淨買賣，不另外查詢），以及同期主力（`broker_flow`）的累計買賣超。
//! - 持股（含 ETF）近 7 天內填息的除權息，與尚未填息、離填息價最遠的幾檔（見 [`crate::domain::dividend::fill`]）。
//! - 下週持股（含 ETF）的除權息日、每股股利、預估入帳與近 5 年填息，下週發放的持股股利入帳，以及
//!   本月已入帳／待入帳的現金股利與去年同月對照（見 [`upcoming_dividend`]）。
//! - 持股的基本面警示：營收連續年減、三率三降、營業虧損、自由現金流為負（見 [`crate::domain::portfolio::health`]）。
//! - ETF 資金流向：全市場淨申購流入、流出最多的 ETF，與持有 ETF 的淨申購、折溢價週平均（見 [`etf`]）。
//! - 有設定策略換股通知的條件時，策略這一期的換股日、本期至今報酬與 0050 對照、最好與最差的持股。
//! - 最近一次策略優勢滾動檢查（每月第一個交易日，見 [`super::strategy_edge`]）的近期與全期數字。
//!
//! 各段各自查詢，某段失敗只記錄警告並略過；都沒有資料時不發送。各段的查詢與文字在子模組
//! [`market`]、[`industry`]、[`crossing`]、[`flows`]、[`holding`]、[`holding_flow`]、[`dividend`]、[`fundamentals`]、[`etf`]、
//! [`strategy`]，組訊息在 [`message`]，這裡只負責排程與
//! 查詢順序。

mod allocation;
mod breakout;
mod crossing;
mod dividend;
mod etf;
mod flows;
mod fundamentals;
mod holding;
mod holding_flow;
mod industry;
mod market;
mod message;
mod strategy;
mod upcoming_dividend;

use crate::core::util::datetime::today;
use jiff::civil::Date;
use std::collections::BTreeSet;

use anyhow::{Context, Result};

use self::{
    allocation::{AllocationSection, allocation_sections},
    breakout::{BreakoutSection, breakout_section},
    crossing::{CrossingSection, crossing_section},
    dividend::{HoldingFill, holding_fills},
    etf::{EtfWeekSection, etf_section},
    flows::{FlowSection, flow_section},
    holding::{HoldingSection, holding_moves},
    holding_flow::{broker_flow_section, holding_flow_section},
    industry::{IndustrySection, industry_section},
    market::{RegimeSection, month_seasonality, regime_section, valuation_level},
    message::{MarketExtras, build_holdings_message, build_message, week_data_date},
    strategy::{StrategySection, strategy_section},
    upcoming_dividend::{UpcomingSection, load_upcoming},
};
use super::holdings;
use crate::{
    app::{calculation::upcoming_dividends::total_shares, notify},
    core::{alert, notify_topic::Topic},
    domain::{
        chip::flow::{self, ChipFlowQuery, FlowPeriod, StockFlow},
        market_index::{
            breadth::BreadthQuery, regime::IndexCloseQuery, valuation::MarketValuationRepository,
        },
        portfolio::{
            chip::HoldingBrokerQuery, health::HoldingFundamentalsQuery, view::HoldingView,
        },
        quote::{
            forward_return::{ForwardReturnRepository, MarketSeasonalityQuery},
            relative_strength::{
                RelativeStrengthCrossingQuery, RelativeStrengthQuery, ScoredUniverseQuery,
            },
            technical::{RankedSignalQuery, TechnicalSignalQuery},
        },
    },
    infra::database::repository::{
        chip::PgChipRepository, dividend::PgDividendRepository,
        dividend_fill::PgDividendFillRepository, etf::PgEtfRepository,
        market_index::PgMarketIndexRepository, market_valuation::PgMarketValuationRepository,
        portfolio::PgPortfolioRepository, portfolio_target::PgPortfolioTargetRepository,
        quote::PgQuoteRepository,
    },
};

/// 一週的交易日數。
const WEEK_DAYS: usize = 5;

/// 每一項列出的產業或股票數。
const TOP: usize = 3;

/// 要統計的法人別與名稱。
const FLOW_INVESTORS: [(&str, &str); 2] = [("foreign", "外資"), ("trust", "投信")];

/// 市場段的資料（所有訂閱者共用）；法人個股淨買賣也留給持股段篩出範圍內的股票，不另外查詢。
#[derive(Debug, Default)]
struct MarketWeek {
    market: Option<RegimeSection>,
    industries: Option<IndustrySection>,
    breakouts: Option<BreakoutSection>,
    crossings: Option<CrossingSection>,
    flows: Vec<FlowSection>,
    /// 各法人的期間與個股淨買賣（依 [`FLOW_INVESTORS`] 順序）。
    investor_rows: Vec<(&'static str, FlowPeriod, Vec<StockFlow>)>,
}

impl MarketWeek {
    fn data_date(&self, etfs: Option<&EtfWeekSection>) -> Option<Date> {
        week_data_date(
            self.market.as_ref(),
            self.industries.as_ref(),
            self.crossings.as_ref(),
            &self.flows,
            etfs,
        )
    }
}

/// 持股段的資料來源（依收件者範圍查詢）。
struct HoldingSources<'a, Q, P> {
    quote: &'a Q,
    portfolio: &'a P,
}

/// 排程入口：市場段（`market.weekly`）送給所有訂閱者；持股段（`holdings.weekly`）依收件者範圍各自產生
/// （會員系統計畫 7.3 混合型通知拆段）。
///
/// # Errors
///
/// 目前不會回傳錯誤；各段失敗只記錄警告並略過。
pub async fn execute() -> Result<()> {
    let date = today();
    let strategy = strategy_section().await;
    // 市場段的 ETF 只用全市場淨申購：不帶任何持有代號。
    let market_etfs = etf_section(&PgEtfRepository::new(), &BTreeSet::new(), date)
        .await
        .unwrap_or_else(|why| {
            tracing::warn!("每週市場摘要：讀取 ETF 資金流向失敗: {why:#}");
            None
        });
    let week = market_week(
        &PgQuoteRepository::new(),
        &PgChipRepository::new(),
        &PgMarketIndexRepository::new(),
        &PgMarketValuationRepository::new(),
        date,
    )
    .await;
    if let Some(message) = market_message(&week, market_etfs.as_ref(), strategy.as_ref()) {
        alert::notify_market(alert::Topic::MarketWeekly, &message).await;
    }

    let data_date = week.data_date(market_etfs.as_ref());
    let dedup_key = format!("{}:{date}", Topic::HoldingsWeekly.code());
    notify::notify_holdings(Topic::HoldingsWeekly, &dedup_key, |view| {
        let week = &week;
        async move {
            let quote = PgQuoteRepository::new();
            let portfolio = PgPortfolioRepository::new();
            let sources = HoldingSources {
                quote: &quote,
                portfolio: &portfolio,
            };
            let data = load_holding_data(&view, date).await?;
            Ok(holdings_message(&sources, week, data_date, date, &data).await)
        }
    })
    .await;
    Ok(())
}

/// 一個範圍的持股資料：普通股代號與依範圍查出的填息、下週除權息、持有 ETF 與配置偏離。
#[derive(Debug, Default)]
struct HoldingData {
    symbols: BTreeSet<String>,
    fills: Vec<HoldingFill>,
    etfs: Option<EtfWeekSection>,
    allocation: Vec<AllocationSection>,
    upcoming: Option<UpcomingSection>,
}

/// 依收件者範圍查持股資料；查不到持股時回錯誤（這個範圍不發），其餘各段失敗只少那一段。
async fn load_holding_data(view: &HoldingView, date: Date) -> Result<HoldingData> {
    let symbols = holdings::common_stock_holdings(view)
        .await
        .context("fetch holdings for weekly holdings summary failed")?;
    // 填息、下週除權息與 ETF 看全部持股（含 ETF）。
    let shares = total_shares(holdings::active_holding_quantities(view).await?);
    let all: BTreeSet<String> = shares.keys().cloned().collect();
    let fills = holding_fills(&PgDividendFillRepository::new(), &all, date).await;
    let upcoming = load_upcoming(
        &PgDividendRepository::new(),
        &PgPortfolioRepository::new(),
        view,
        &shares,
        date,
    )
    .await;
    let etfs = etf_section(&PgEtfRepository::new(), &all, date)
        .await
        .unwrap_or_else(|why| {
            tracing::warn!("每週持股摘要：讀取持有 ETF 失敗: {why:#}");
            None
        });
    let target_repo = PgPortfolioTargetRepository::new();
    let allocation = allocation_sections(&target_repo, &target_repo, view, date).await;
    Ok(HoldingData {
        symbols,
        fills,
        etfs,
        allocation,
        upcoming,
    })
}

/// 查詢市場段各段；某段失敗只記錄警告。這裡不接受任何持股來源，市場段因此無法讀到持股（7.3 防漏）。
async fn market_week(
    quote: &(
         impl ScoredUniverseQuery
         + RelativeStrengthCrossingQuery
         + ForwardReturnRepository
         + MarketSeasonalityQuery
         + RankedSignalQuery
         + TechnicalSignalQuery
     ),
    chip: &impl ChipFlowQuery,
    index: &(impl IndexCloseQuery + BreadthQuery),
    valuation: &impl MarketValuationRepository,
    date: Date,
) -> MarketWeek {
    let mut market = regime_section(index, date).await.unwrap_or_else(|why| {
        tracing::warn!("每週市場摘要：讀取加權指數失敗: {why:#}");
        None
    });
    if let Some(section) = market.as_mut() {
        section.valuation = valuation_level(valuation, section.state.date)
            .await
            .unwrap_or_else(|why| {
                tracing::warn!("每週市場摘要：讀取估值水位失敗: {why:#}");
                None
            });
        section.seasonality = match month_seasonality(quote, section.state.date).await {
            Ok(seasonality) => Some(seasonality),
            Err(why) => {
                tracing::warn!("每週市場摘要：讀取本月季節性失敗: {why:#}");
                None
            }
        };
    }
    let industries = industry_section(quote, date).await.unwrap_or_else(|why| {
        tracing::warn!("每週市場摘要：讀取產業相對強度失敗: {why:#}");
        None
    });
    let breakouts = breakout_section(quote, date).await.unwrap_or_else(|why| {
        tracing::warn!("每週市場摘要：讀取區間突破失敗: {why:#}");
        None
    });
    let crossings = crossing_section(quote, date).await.unwrap_or_else(|why| {
        tracing::warn!("每週市場摘要：讀取相對強度穿越失敗: {why:#}");
        None
    });
    let mut flows = Vec::new();
    let mut investor_rows = Vec::new();
    for (investor, label) in FLOW_INVESTORS {
        match flow::load_flows(chip, date, WEEK_DAYS, investor).await {
            Ok(Some((period, rows))) => {
                investor_rows.push((label, period, rows.clone()));
                flows.push(flow_section(label, period, rows));
            }
            Ok(None) => {}
            Err(why) => tracing::warn!("每週市場摘要：讀取{label}產業買賣超失敗: {why:#}"),
        }
    }
    tracing::info!(
        "每週市場摘要: industries={} flows={}",
        industries
            .as_ref()
            .map_or(0, |section| section.industries.len()),
        flows.len()
    );
    MarketWeek {
        market,
        industries,
        breakouts,
        crossings,
        flows,
        investor_rows,
    }
}

/// 組出市場段；都沒有資料時回 `None`。
fn market_message(
    week: &MarketWeek,
    etfs: Option<&EtfWeekSection>,
    strategy: Option<&StrategySection>,
) -> Option<String> {
    build_message(
        week.market.as_ref(),
        week.industries.as_ref(),
        week.crossings.as_ref(),
        &week.flows,
        MarketExtras {
            breakouts: week.breakouts.as_ref(),
            etfs,
        },
        strategy,
        holdings::stock_name,
    )
}

/// 依一個範圍的持股組出持股段；範圍內沒有任何持股內容時回 `None`。
async fn holdings_message<Q, P>(
    sources: &HoldingSources<'_, Q, P>,
    week: &MarketWeek,
    data_date: Option<Date>,
    date: Date,
    data: &HoldingData,
) -> Option<String>
where
    Q: RelativeStrengthQuery,
    P: HoldingFundamentalsQuery + HoldingBrokerQuery,
{
    let symbols = &data.symbols;
    let mut holding_flows = Vec::new();
    // 主力一段沿用第一個法人的期間，並借它的成交量與收盤價算佔量、排序。
    let mut broker_basis = None;
    for (label, period, rows) in &week.investor_rows {
        holding_flows.push(holding_flow_section(label, *period, rows, symbols));
        if broker_basis.is_none() {
            let held: Vec<StockFlow> = rows
                .iter()
                .filter(|row| symbols.contains(&row.stock_symbol))
                .cloned()
                .collect();
            broker_basis = Some((*period, held));
        }
    }
    let symbol_list: Vec<String> = symbols.iter().cloned().collect();
    if let Some((period, rows)) = &broker_basis {
        match sources
            .portfolio
            .fetch_broker_period(period.from, period.data_date, &symbol_list)
            .await
        {
            Ok(nets) => holding_flows.push(broker_flow_section(*period, &nets, rows)),
            Err(why) => tracing::warn!("每週持股摘要：讀取持股主力進出失敗: {why:#}"),
        }
    }
    let moves = match sources
        .quote
        .fetch_relative_strength(date, WEEK_DAYS as u16 + 1, &symbol_list)
        .await
    {
        Ok(days) => holding_moves(&days),
        Err(why) => {
            tracing::warn!("每週持股摘要：讀取持股相對強度失敗: {why:#}");
            Vec::new()
        }
    };
    let fundamentals = sources
        .portfolio
        .fetch_holding_fundamentals(&symbol_list)
        .await
        .unwrap_or_else(|why| {
            tracing::warn!("每週持股摘要：讀取持股基本面失敗: {why:#}");
            Vec::new()
        });
    build_holdings_message(
        data_date,
        HoldingSection {
            moves: &moves,
            flows: &holding_flows,
            fills: &data.fills,
            fundamentals: &fundamentals,
            etfs: data.etfs.as_ref(),
            allocation: &data.allocation,
            upcoming: data.upcoming.as_ref(),
        },
        holdings::stock_name,
    )
}

/// 測試用：市場段與持股段依序接成一則（舊版單一訊息的內容），各段測試沿用同一組替身。
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
async fn run(
    quote: &(
         impl ScoredUniverseQuery
         + RelativeStrengthQuery
         + RelativeStrengthCrossingQuery
         + ForwardReturnRepository
         + MarketSeasonalityQuery
         + RankedSignalQuery
         + TechnicalSignalQuery
     ),
    chip: &impl ChipFlowQuery,
    index: &(impl IndexCloseQuery + BreadthQuery),
    valuation: &impl MarketValuationRepository,
    portfolio: &(impl HoldingFundamentalsQuery + HoldingBrokerQuery),
    date: Date,
    symbols: &BTreeSet<String>,
    strategy: Option<&StrategySection>,
    fills: &[HoldingFill],
    etfs: Option<&EtfWeekSection>,
    allocation: &[AllocationSection],
    upcoming: Option<&UpcomingSection>,
) -> Option<String> {
    let week = market_week(quote, chip, index, valuation, date).await;
    let market = market_message(&week, etfs, strategy);
    let data = HoldingData {
        symbols: symbols.clone(),
        fills: fills.to_vec(),
        etfs: etfs.cloned(),
        allocation: allocation.to_vec(),
        upcoming: upcoming.cloned(),
    };
    let sources = HoldingSources { quote, portfolio };
    let held = holdings_message(&sources, &week, week.data_date(etfs), date, &data).await;
    match (market, held) {
        (Some(market), Some(held)) => Some(format!("{market}{held}")),
        (market, held) => market.or(held),
    }
}

/// 帶正負號的分數變化文字。
pub(super) fn signed(value: f64) -> String {
    format!("{value:+.1}")
}

/// 億元文字（帶正負號、一位小數）。
pub(super) fn hundred_million(amount: f64) -> String {
    format!("{:+.1} 億", amount / 1e8)
}

#[cfg(test)]
mod tests;
