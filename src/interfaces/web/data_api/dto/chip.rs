//! `/stocks/{symbol}/chip` 的 request、response 與 OpenAPI schema。
//!
//! 一次回傳單一股票的籌碼面：每日三大法人與融資融券、外資投信連續買賣超天數、
//! 集保千張大戶週資料、最新月份董監持股與設質，以及最近一天的券商分點主力進出。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 籌碼 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct ChipParams {
    /// 每日籌碼回傳的交易日數，預設 20，範圍 1–120。
    #[param(minimum = 1, maximum = 120, default = 20)]
    pub(crate) days: Option<u16>,
}

/// 單一交易日的三大法人買賣超與融資融券餘額。
///
/// 法人單位是「股」、融資融券單位是「張」（與交易所公告一致）；
/// 來源當天沒有這檔股票的那一類資料時為 `null`（例如不能信用交易的股票沒有融資融券）。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ChipDay {
    /// 交易日，格式 `YYYY-MM-DD`。
    pub(crate) date: String,
    /// 外資及陸資（含外資自營商）買賣超股數。
    pub(crate) foreign_net: Option<i64>,
    /// 投信買賣超股數。
    pub(crate) trust_net: Option<i64>,
    /// 自營商（自行買賣與避險合計）買賣超股數。
    pub(crate) dealer_net: Option<i64>,
    /// 三大法人合計買賣超股數。
    pub(crate) total_net: Option<i64>,
    /// 融資餘額（張）。
    pub(crate) margin_balance: Option<i64>,
    /// 融資餘額較前一日增減（張）。
    pub(crate) margin_change: Option<i64>,
    /// 融券餘額（張）。
    pub(crate) short_balance: Option<i64>,
    /// 融券餘額較前一日增減（張）。
    pub(crate) short_change: Option<i64>,
}

/// 外資、投信從最新一天往回數的連續同向買賣超天數。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct ChipStreak {
    /// 外資連續天數；正值為連續買超、負值為連續賣超、0 為最新一天沒有買賣超或沒有資料。
    pub(crate) foreign_days: i64,
    /// 投信連續天數，正負意義同 `foreign_days`。
    pub(crate) trust_days: i64,
}

/// 集保戶股權分散的單週摘要。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct HolderWeek {
    /// 資料日期，格式 `YYYY-MM-DD`。
    pub(crate) date: String,
    /// 千張大戶（1,000,001 股以上）人數。
    pub(crate) major_holders: i64,
    /// 千張大戶持股佔集保庫存比例（%）。
    pub(crate) major_percent: Option<f64>,
    /// 集保股東總人數。
    pub(crate) total_holders: i64,
}

/// 最新月份的董監事持股與設質合計。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct InsiderSummary {
    /// 資料月份，格式 `YYYY-MM`。
    pub(crate) month: String,
    /// 申報的內部人列數（同一人不同職稱分列）。
    pub(crate) insiders: i64,
    /// 合計持股（股）。
    pub(crate) shares: i64,
    /// 合計設質股數（股）。
    pub(crate) pledged: i64,
    /// 內部人關係人合計設質股數（股）。
    pub(crate) related_pledged: i64,
    /// 設質比例（設質 ÷ 持股，%）；持股為 0 時為 `null`。
    pub(crate) pledge_percent: Option<f64>,
}

/// 單一券商分點的買賣超（張）。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BrokerNet {
    /// 券商分點名稱。
    pub(crate) name: String,
    /// 買進張數。
    pub(crate) buy: i64,
    /// 賣出張數。
    pub(crate) sell: i64,
    /// 買超（或賣超）張數，一律為非負值，方向由所在清單決定。
    pub(crate) net: i64,
    /// 佔成交比重（%）。
    pub(crate) share: Option<f64>,
}

/// 最近一天的券商分點主力進出（只有持股才有資料）。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BrokerFlowDay {
    /// 資料日期，格式 `YYYY-MM-DD`。
    pub(crate) date: String,
    /// 主力買賣超張數（合計買超 − 合計賣超），正值為買超。
    pub(crate) main_net: i64,
    /// 主力買賣超佔成交量比重（%），正值為買超。
    pub(crate) main_share: Option<f64>,
    /// 買超前幾名分點。
    pub(crate) buyers: Vec<BrokerNet>,
    /// 賣超前幾名分點。
    pub(crate) sellers: Vec<BrokerNet>,
}

/// 籌碼 endpoint 的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ChipResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 每日籌碼最新一天（`YYYY-MM-DD`）；沒有資料時為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// 每日籌碼，依日期由新到舊。
    pub(crate) daily: Vec<ChipDay>,
    /// 外資、投信連續買賣超天數（只在回傳的交易日內計算）。
    pub(crate) streak: ChipStreak,
    /// 最近 8 週的千張大戶資料，由新到舊。
    pub(crate) holder_distribution: Vec<HolderWeek>,
    /// 最新月份的董監持股與設質；沒有資料時為 `null`。
    pub(crate) insider: Option<InsiderSummary>,
    /// 最近一天的主力進出；不是持股或尚無資料時為 `null`。
    pub(crate) broker_flow: Option<BrokerFlowDay>,
}
