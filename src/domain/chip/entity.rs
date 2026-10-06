//! 籌碼資料實體：三大法人、融資融券、集保股權分散、董監持股與設質、券商分點主力進出。

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// 單一股票單日的三大法人買賣超（單位：股）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstitutionalNet {
    /// 股票代號。
    pub stock_symbol: String,
    /// 外資及陸資（含外資自營商）。
    pub foreign: i64,
    /// 投信。
    pub trust: i64,
    /// 自營商（自行買賣與避險合計）。
    pub dealer: i64,
}

/// 單一股票單日的融資融券餘額（單位：張）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarginRecord {
    /// 股票代號。
    pub stock_symbol: String,
    /// 前日融資餘額。
    pub margin_previous: i64,
    /// 今日融資餘額。
    pub margin_balance: i64,
    /// 前日融券餘額。
    pub short_previous: i64,
    /// 今日融券餘額。
    pub short_balance: i64,
}

/// 單一證券一週的股權分散摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolderDistributionRecord {
    /// 資料日期。
    pub date: NaiveDate,
    /// 證券代號。
    pub stock_symbol: String,
    /// 千張大戶人數。
    pub major_holders: i64,
    /// 千張大戶持股佔集保庫存比例（%）。
    pub major_percent: Decimal,
    /// 集保股東總人數。
    pub total_holders: i64,
}

/// 單一內部人在單一職稱下的月持股與設質。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsiderHoldingRecord {
    /// 資料月份（該月 1 日）。
    pub month: NaiveDate,
    /// 股票代號。
    pub stock_symbol: String,
    /// 職稱。
    pub title: String,
    /// 姓名。
    pub name: String,
    /// 目前持股（股）。
    pub shares: i64,
    /// 設質股數（股）。
    pub pledged: i64,
    /// 內部人關係人設質股數（股）。
    pub related_pledged: i64,
}

/// 單一券商分點的買賣超（單位：張）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerNetRecord {
    /// 券商分點名稱。
    pub name: String,
    /// 買進張數。
    pub buy: i64,
    /// 賣出張數。
    pub sell: i64,
    /// 買超（或賣超）張數，一律為非負值；方向由所在的那一側決定。
    pub net: i64,
    /// 佔成交比重（%）。
    pub share: Decimal,
}

/// 單一股票單日的主力進出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerFlowRecord {
    /// 資料日期。
    pub date: NaiveDate,
    /// 股票代號。
    pub stock_symbol: String,
    /// 合計買超張數。
    pub buy_total: i64,
    /// 合計賣超張數。
    pub sell_total: i64,
    /// 主力買賣超佔成交量比重（%），正值為買超。
    pub main_share: Decimal,
    /// 買超前幾名。
    pub buyers: Vec<BrokerNetRecord>,
    /// 賣超前幾名。
    pub sellers: Vec<BrokerNetRecord>,
}
