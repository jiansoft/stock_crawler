//! Data API 讀取 ETF 專區資料的查詢合約：清單篩選與排序、單檔明細。
//!
//! 資料都已由排程預先整理（`etf_snapshot`、`etf_performance`、`etf_profile`、持股），這裡只讀不算。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use super::EtfHoldingPeriod;

/// 清單可用的排序鍵；以白名單對應欄位，避免把使用者輸入拼進 SQL。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EtfSort {
    /// 成交值。
    TradeValue,
    /// 資產規模。
    TotalAssets,
    /// 折溢價。
    PremiumRate,
    /// 近 12 個月殖利率。
    Yield12m,
    /// 淨申購金額。
    NetFlow,
    /// 近 1 月含息報酬。
    Ret1m,
    /// 今年以來含息報酬。
    RetYtd,
    /// 近 1 年含息報酬。
    Ret1y,
    /// 受益人數。
    Holders,
}

impl EtfSort {
    /// 由 API 參數解析；不在白名單內回傳 `None`。
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "trade_value" => Self::TradeValue,
            "total_assets" => Self::TotalAssets,
            "premium_rate" => Self::PremiumRate,
            "yield_12m" => Self::Yield12m,
            "net_flow" => Self::NetFlow,
            "ret_1m" => Self::Ret1m,
            "ret_ytd" => Self::RetYtd,
            "ret_1y" => Self::Ret1y,
            "holders" => Self::Holders,
            _ => return None,
        })
    }

    /// 對應的 `etf_snapshot` 欄位名稱。
    pub fn column(self) -> &'static str {
        match self {
            Self::TradeValue => "trade_value",
            Self::TotalAssets => "total_assets",
            Self::PremiumRate => "premium_rate",
            Self::Yield12m => "yield_12m",
            Self::NetFlow => "net_flow",
            Self::Ret1m => "ret_1m",
            Self::RetYtd => "ret_ytd",
            Self::Ret1y => "ret_1y",
            Self::Holders => "holders",
        }
    }
}

/// 清單的篩選、排序與筆數。
#[derive(Debug, Clone, PartialEq)]
pub struct EtfListFilter {
    /// 分類代碼（見 [`super::Strategy::code`]）；空集合為不限。
    pub strategies: Vec<String>,
    /// 投資地區；`None` 為不限。
    pub region: Option<String>,
    /// 配息頻率；`None` 為不限。
    pub frequency: Option<String>,
    /// 市場代碼（2 上市、4 上櫃）；`None` 為不限。
    pub market_id: Option<i32>,
    /// 代號、名稱、全名、發行投信、追蹤指數或題材的關鍵字；`None` 為不限。
    pub keyword: Option<String>,
    /// 排序鍵。
    pub sort: EtfSort,
    /// 是否由大到小。
    pub descending: bool,
    /// 最多筆數。
    pub limit: i64,
}

/// 清單與明細共用的一檔 ETF 概況（取自 `etf_snapshot`）；比率為 %，金額為元。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EtfOverview {
    /// ETF 代號。
    pub stock_symbol: String,
    /// 名稱。
    pub name: String,
    /// 市場代碼（2 上市、4 上櫃）。
    pub market_id: i32,
    /// 分類代碼。
    pub strategy: String,
    /// 投資地區。
    pub region: String,
    /// 題材。
    pub themes: Vec<String>,
    /// 配息頻率。
    pub dividend_frequency: String,
    /// 發行投信。
    pub issuer: String,
    /// 追蹤指數。
    pub benchmark: String,
    /// 行情日。
    pub quote_date: Option<NaiveDate>,
    /// 收盤價。
    pub closing_price: Option<f64>,
    /// 漲跌幅（%）。
    pub change_range: Option<f64>,
    /// 成交值（元）。
    pub trade_value: Option<f64>,
    /// 淨值日期。
    pub nav_date: Option<NaiveDate>,
    /// 預估淨值。
    pub nav: Option<f64>,
    /// 折溢價（%）。
    pub premium_rate: Option<f64>,
    /// 近 20 筆折溢價平均（%）。
    pub premium_avg_20d: Option<f64>,
    /// 資產規模（元）。
    pub total_assets: Option<f64>,
    /// 淨申購金額（元）。
    pub net_flow: Option<f64>,
    /// 近 12 個月現金配息合計。
    pub dividend_12m: Option<f64>,
    /// 近 12 個月殖利率（%）。
    pub yield_12m: Option<f64>,
    /// 下次除息日。
    pub next_ex_date: Option<NaiveDate>,
    /// 下次配息金額。
    pub next_dividend: Option<f64>,
    /// 近 1 月含息報酬（%）。
    pub ret_1m: Option<f64>,
    /// 今年以來含息報酬（%）。
    pub ret_ytd: Option<f64>,
    /// 近 1 年含息報酬（%）。
    pub ret_1y: Option<f64>,
    /// 集保受益人數。
    pub holders: Option<i64>,
    /// 外資買賣超（股）。
    pub foreign_net: Option<i64>,
}

/// 基本資料（取自 `etf_profile`）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EtfProfileView {
    /// 基金全名。
    pub full_name: String,
    /// 成立日。
    pub inception_date: Option<NaiveDate>,
    /// 是否為主動式。
    pub is_active: bool,
    /// 經理費（來源原文）。
    pub management_fee: String,
    /// 保管費（來源原文）。
    pub custodian_fee: String,
    /// 計價幣別。
    pub currency: String,
    /// 資料來源：yahoo、heuristic。
    pub source: String,
}

/// 績效（取自 `etf_performance`）；報酬為 %，1 年以上年化。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EtfPerformanceView {
    /// 市價含息報酬的基準日。
    pub calc_date: Option<NaiveDate>,
    /// 市價含息報酬：1 週。
    pub ret_1w: Option<f64>,
    /// 1 個月。
    pub ret_1m: Option<f64>,
    /// 3 個月。
    pub ret_3m: Option<f64>,
    /// 6 個月。
    pub ret_6m: Option<f64>,
    /// 今年以來。
    pub ret_ytd: Option<f64>,
    /// 1 年。
    pub ret_1y: Option<f64>,
    /// 3 年年化。
    pub ret_3y: Option<f64>,
    /// 5 年年化。
    pub ret_5y: Option<f64>,
    /// 近 1 年年化波動。
    pub vol_1y: Option<f64>,
    /// 近 1 年最大回撤（負值）。
    pub mdd_1y: Option<f64>,
    /// 近 1 年對 0050 的 beta。
    pub beta_1y: Option<f64>,
    /// 近 1 年夏普值。
    pub sharpe_1y: Option<f64>,
    /// 淨值含息報酬的基準日。
    pub nav_calc_date: Option<NaiveDate>,
    /// 淨值含息報酬：1 個月。
    pub nav_ret_1m: Option<f64>,
    /// 淨值含息報酬：今年以來。
    pub nav_ret_ytd: Option<f64>,
    /// 淨值含息報酬：1 年。
    pub nav_ret_1y: Option<f64>,
    /// 淨值含息報酬：3 年年化。
    pub nav_ret_3y: Option<f64>,
}

/// 持股或配置的一列。
#[derive(Debug, Clone, PartialEq)]
pub struct EtfWeightRow {
    /// 區塊：`holding` 前十大、`industry` 行業比重、`asset` 資產分佈。
    pub kind: String,
    /// 資料日期。
    pub as_of: NaiveDate,
    /// 台股成分股代號（只有前十大持股有，其餘為空字串）。
    pub symbol: String,
    /// 名稱或類別。
    pub label: String,
    /// 比例（%）。
    pub weight: f64,
}

/// 近期每日淨值與折溢價的一列。
#[derive(Debug, Clone, PartialEq)]
pub struct EtfNavDay {
    /// 日期。
    pub date: NaiveDate,
    /// 淨值（正式淨值優先，當天用預估淨值）。
    pub nav: Option<f64>,
    /// 市價。
    pub market_price: Option<f64>,
    /// 折溢價（%）。
    pub premium_rate: Option<f64>,
    /// 單位增減；歷史回補的日子為 `None`。
    pub units_change: Option<i64>,
}

/// 單檔明細。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EtfDetail {
    /// 概況。
    pub overview: EtfOverview,
    /// 基本資料；沒有時為 `None`。
    pub profile: Option<EtfProfileView>,
    /// 績效；沒有時為 `None`。
    pub performance: Option<EtfPerformanceView>,
    /// 前十大持股、行業比重與資產分佈，依區塊與來源順序排列。
    pub weights: Vec<EtfWeightRow>,
    /// 近期每日淨值，由新到舊。
    pub navs: Vec<EtfNavDay>,
}

/// 一檔 ETF 近幾期的前十大持股。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EtfHoldingHistory {
    /// ETF 代號。
    pub stock_symbol: String,
    /// 名稱。
    pub name: String,
    /// 每一期的前十大持股，由新到舊。
    pub periods: Vec<EtfHoldingPeriod>,
}

/// ETF 專區的讀取合約。
#[async_trait]
pub trait EtfQueryRepository: Send + Sync {
    /// 依條件篩選、排序後的清單與符合的總檔數。
    async fn list_etfs(&self, filter: &EtfListFilter) -> Result<(i64, Vec<EtfOverview>)>;

    /// 單檔明細；不是上市櫃 ETF 時回傳 `None`。`nav_days` 為近期淨值的筆數。
    async fn etf_detail(&self, stock_symbol: &str, nav_days: i64) -> Result<Option<EtfDetail>>;

    /// 最近 `periods` 期的前十大持股（`etf_holding_history`）；不是上市櫃 ETF 時回傳 `None`。
    async fn holding_history(
        &self,
        stock_symbol: &str,
        periods: i64,
    ) -> Result<Option<EtfHoldingHistory>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_whitelist_round_trips() {
        for key in [
            "trade_value",
            "total_assets",
            "premium_rate",
            "yield_12m",
            "net_flow",
            "ret_1m",
            "ret_ytd",
            "ret_1y",
            "holders",
        ] {
            assert_eq!(EtfSort::parse(key).map(EtfSort::column), Some(key));
        }
        assert_eq!(EtfSort::parse("name; DROP TABLE"), None);
    }
}
