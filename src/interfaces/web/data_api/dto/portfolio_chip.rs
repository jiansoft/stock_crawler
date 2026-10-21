//! 持股籌碼日報 endpoint（`/portfolio/chip`）的參數與回應。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 持股籌碼日報的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct PortfolioChipParams {
    /// 查詢日（YYYY-MM-DD）；取不晚於它的最近交易日，省略為最新交易日。
    pub(crate) date: Option<String>,
}

/// 單一分點的買賣超（張）。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct PortfolioChipBrokerNet {
    /// 券商分點名稱。
    pub(crate) name: String,
    /// 買進張數。
    pub(crate) buy: i64,
    /// 賣出張數。
    pub(crate) sell: i64,
    /// 買超（或賣超）張數，非負值；方向由所在的一側決定。
    pub(crate) net: i64,
    /// 佔成交比重（%）。
    pub(crate) share: f64,
}

/// 持股當天的主力進出（張）。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct PortfolioChipBroker {
    /// 主力買賣超張數（買超為正）。
    pub(crate) main_net: i64,
    /// 主力買賣超佔成交量比重（%），正值為買超。
    pub(crate) main_share: f64,
    /// 買超前幾名分點。
    pub(crate) buyers: Vec<PortfolioChipBrokerNet>,
    /// 賣超前幾名分點。
    pub(crate) sellers: Vec<PortfolioChipBrokerNet>,
}

/// 持股近幾個交易日的主力累計（張）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct PortfolioChipBrokerRecent {
    /// 主力買賣超合計（張），正值為買超。
    pub(crate) main_net: i64,
    /// 主力買超的天數。
    pub(crate) buy_days: i64,
    /// 主力賣超的天數。
    pub(crate) sell_days: i64,
}

/// 單一持股當天的籌碼；張數皆已由股換算成張（四捨五入），來源沒有資料時為 `null`。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct PortfolioChipItem {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 收盤價。
    pub(crate) closing_price: Option<f64>,
    /// 成交量（張）。
    pub(crate) volume: Option<i64>,
    /// 外資買賣超（張）。
    pub(crate) foreign_net: Option<i64>,
    /// 投信買賣超（張）。
    pub(crate) trust_net: Option<i64>,
    /// 自營商買賣超（張）。
    pub(crate) dealer_net: Option<i64>,
    /// 外資買賣超佔成交量（%，帶正負號）；沒有成交量為 `null`。
    pub(crate) foreign_share: Option<f64>,
    /// 投信買賣超佔成交量（%）。
    pub(crate) trust_share: Option<f64>,
    /// 外資連續買賣超天數（正連買、負連賣，0 為當天沒有）。
    pub(crate) foreign_streak: i32,
    /// 投信連續買賣超天數。
    pub(crate) trust_streak: i32,
    /// 融資餘額（張）。
    pub(crate) margin_balance: Option<i64>,
    /// 融資增減（張）。
    pub(crate) margin_change: Option<i64>,
    /// 融券餘額（張）。
    pub(crate) short_balance: Option<i64>,
    /// 融券增減（張）。
    pub(crate) short_change: Option<i64>,
    /// 主力進出；當天沒有（2026-10-07 以前或來源只有持股）為 `null`。
    pub(crate) broker: Option<PortfolioChipBroker>,
    /// 含當天在內最近 `broker_recent_days` 個有主力資料的交易日的主力累計；期間內沒有資料為 `null`。
    pub(crate) broker_recent: Option<PortfolioChipBrokerRecent>,
    /// 達到 Telegram 通知門檻的原因代碼：`foreign_heavy`、`trust_heavy`、`foreign_streak`、
    /// `trust_streak`、`margin`、`short`、`broker`。
    pub(crate) reasons: Vec<String>,
}

/// 持股籌碼日報。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct PortfolioChipResponse {
    /// 交易日（不晚於查詢日、最近一個有籌碼資料的日子）；完全沒有資料為 `null`。
    pub(crate) date: Option<String>,
    /// 前一個有籌碼資料的交易日。
    pub(crate) previous_date: Option<String>,
    /// 後一個有籌碼資料的交易日。
    pub(crate) next_date: Option<String>,
    /// 達通知門檻的持股檔數。
    pub(crate) notable: usize,
    /// `broker_recent` 涵蓋的主力交易日數（最多 5，含當天；沒有主力資料為 0）。
    pub(crate) broker_recent_days: usize,
    /// 當天收盤時仍持有的普通股（不含 ETF），依代號排序。
    pub(crate) items: Vec<PortfolioChipItem>,
}
