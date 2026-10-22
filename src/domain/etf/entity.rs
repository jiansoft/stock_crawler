use chrono::{DateTime, FixedOffset, NaiveDate};
use rust_decimal::Decimal;

/// 一檔 ETF 某一天的淨值、折溢價與受益單位（證交所 MIS 預估淨值）。
#[derive(Debug, Clone, PartialEq)]
pub struct EtfNavRecord {
    /// 資料日期。
    pub date: NaiveDate,
    /// ETF 代號。
    pub stock_symbol: String,
    /// 已發行受益權單位數。
    pub outstanding_units: i64,
    /// 與前一營業日已發行單位數的差異（正為淨申購）。
    pub units_change: i64,
    /// 資料時間的成交價；無成交為 `None`。
    pub market_price: Option<Decimal>,
    /// 投信預估淨值。
    pub estimated_nav: Option<Decimal>,
    /// 預估折溢價幅度（%）。
    pub premium_rate: Option<Decimal>,
    /// 前一營業日的正式單位淨值，用來回填前一筆的 `nav`。
    pub previous_nav: Option<Decimal>,
    /// 來源資料時間。
    pub source_time: Option<DateTime<FixedOffset>>,
}

/// 一檔 ETF 某一天的正式淨值與收盤市價（MoneyDJ 歷史資料，用來回補 MIS 上線前的日子）。
#[derive(Debug, Clone, PartialEq)]
pub struct EtfNavHistoryPoint {
    /// 資料日期。
    pub date: NaiveDate,
    /// 正式單位淨值。
    pub nav: Decimal,
    /// 收盤市價；來源沒有時為 `None`。
    pub market_price: Option<Decimal>,
}

/// ETF 基本資料與分類。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EtfProfileRecord {
    /// ETF 代號。
    pub stock_symbol: String,
    /// 基金全名。
    pub full_name: String,
    /// 發行投信。
    pub issuer: String,
    /// 成立日。
    pub inception_date: Option<NaiveDate>,
    /// 追蹤指數。
    pub benchmark: String,
    /// 來源的類型（指數型、主動型…）。
    pub category: String,
    /// 是否為主動式。
    pub is_active: bool,
    /// 投資地區。
    pub region: String,
    /// 本站分類代碼（見 [`super::Strategy::code`]）。
    pub strategy: String,
    /// 題材。
    pub themes: Vec<String>,
    /// 配息頻率：monthly、quarterly、semi_annual、annual、none；未知為空字串。
    pub dividend_frequency: String,
    /// 經理費（來源原文）。
    pub management_fee: String,
    /// 保管費（來源原文）。
    pub custodian_fee: String,
    /// 計價幣別。
    pub currency: String,
    /// 交易所。
    pub exchange_name: String,
    /// 經理人。
    pub manager: String,
    /// 網站。
    pub website: String,
    /// 來源公布的資產規模（元）。
    pub total_assets: Option<Decimal>,
    /// 資產規模的資料日期。
    pub total_assets_date: Option<NaiveDate>,
    /// 來源的淨值報酬（%）。
    pub nav_return: Option<serde_json::Value>,
    /// 資料來源：yahoo、heuristic。
    pub source: String,
}

/// 前十大持股的一列。
#[derive(Debug, Clone, PartialEq)]
pub struct EtfHoldingRecord {
    /// 排名（1 起算）。
    pub rank: i16,
    /// 台股成分股代號；債券等沒有代號為空字串。
    pub holding_symbol: String,
    /// 名稱。
    pub holding_name: String,
    /// 比例（%）。
    pub weight: Decimal,
}

/// 一檔 ETF 某一期的前十大持股。
#[derive(Debug, Clone, PartialEq)]
pub struct EtfHoldingPeriod {
    /// 持股資料日期。
    pub as_of: NaiveDate,
    /// 前十大持股，依排名。
    pub holdings: Vec<EtfHoldingRecord>,
}

/// 一檔 ETF 最新一期與前一期的前十大持股。
#[derive(Debug, Clone, PartialEq)]
pub struct EtfHoldingPeriods {
    /// ETF 代號。
    pub stock_symbol: String,
    /// 前一期。
    pub previous: EtfHoldingPeriod,
    /// 最新一期。
    pub current: EtfHoldingPeriod,
}

/// 行業比重或資產分佈的一列。
#[derive(Debug, Clone, PartialEq)]
pub struct EtfAllocationRecord {
    /// 名稱（行業別或資產類別）。
    pub label: String,
    /// 比例（%）。
    pub weight: Decimal,
}

/// 一檔 ETF 最新一期的持股、行業比重與資產分佈。各區塊資料日期不同，沒有資料時日期為 `None`。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EtfPortfolio {
    /// ETF 代號。
    pub stock_symbol: String,
    /// 前十大持股的資料日期。
    pub holdings_as_of: Option<NaiveDate>,
    /// 前十大持股。
    pub holdings: Vec<EtfHoldingRecord>,
    /// 行業比重的資料日期。
    pub industries_as_of: Option<NaiveDate>,
    /// 行業比重。
    pub industries: Vec<EtfAllocationRecord>,
    /// 資產分佈的資料日期。
    pub assets_as_of: Option<NaiveDate>,
    /// 資產分佈。
    pub assets: Vec<EtfAllocationRecord>,
}

/// 一檔 ETF 最近幾個交易日的資金流向與折溢價，供每週市場摘要使用。
#[derive(Debug, Clone, PartialEq)]
pub struct EtfWeeklyFlow {
    /// ETF 代號。
    pub stock_symbol: String,
    /// 名稱（沒有快照時為代號）。
    pub name: String,
    /// 期間淨申購金額（元）：每日單位增減 × 當日淨值的合計，正為淨流入。
    pub net_flow: f64,
    /// 期間折溢價平均（%）。
    pub premium_avg: Option<f64>,
    /// 期間最後一天的折溢價（%）。
    pub premium_last: Option<f64>,
    /// 期間第一個交易日。
    pub from: NaiveDate,
    /// 期間最後一個交易日。
    pub to: NaiveDate,
}

/// 某一天的折溢價，供折溢價通知判斷連續天數。
#[derive(Debug, Clone, PartialEq)]
pub struct PremiumPoint {
    /// 資料日期。
    pub date: NaiveDate,
    /// 折溢價幅度（%）。
    pub premium_rate: f64,
}
