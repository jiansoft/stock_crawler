//! `/stocks/*` 的基本面歷史 request、response 與 OpenAPI schema。
//!
//! 涵蓋月營收、季／年度財報、股利發放與個股估值。所有缺值欄位皆保留
//! `Option`，讓 serde 輸出 JSON `null` 而非猜測成零值。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// OpenAPI 文件使用的財報期間類型。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
enum StatementPeriodTypeValue {
    /// 僅季度資料。
    Quarterly,
    /// 僅年度資料。
    Annual,
    /// 年度與季度資料。
    All,
}

/// 單月營收資料。
///
/// 對應資料表 `"Revenue"` 一列；`month` 由資料庫的 `YYYYMM` 整數（例如
/// `202606`）轉成 `YYYY-MM` 字串，內部編碼不對外暴露。金額與百分比欄位
/// 沿用 §3.1 規則：`NUMERIC` 無法安全轉 `f64` 時輸出 `null`，資料庫中
/// 本來就是 `0` 的值維持 `0`，不推斷成缺值。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct MonthlyRevenue {
    /// 營收月份，格式 `YYYY-MM`。
    pub(crate) month: String,
    /// 當月營收（仟元）。
    pub(crate) monthly_revenue: Option<f64>,
    /// 上月營收（仟元）。
    pub(crate) last_month_revenue: Option<f64>,
    /// 去年同月營收（仟元）。
    pub(crate) last_year_same_month_revenue: Option<f64>,
    /// 當年度累計營收（仟元）。
    pub(crate) monthly_accumulated_revenue: Option<f64>,
    /// 去年同期累計營收（仟元）。
    pub(crate) last_year_monthly_accumulated_revenue: Option<f64>,
    /// 月增率（%）。
    pub(crate) month_over_month_percent: Option<f64>,
    /// 年增率（%）。
    pub(crate) year_over_year_percent: Option<f64>,
    /// 累計年增率（%）。
    pub(crate) accumulated_year_over_year_percent: Option<f64>,
    /// 當月均價（元）。
    pub(crate) average_price: Option<f64>,
    /// 當月最低價（元）。
    pub(crate) lowest_price: Option<f64>,
    /// 當月最高價（元）。
    pub(crate) highest_price: Option<f64>,
}

/// 月營收歷史的成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct MonthlyRevenueResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 實際回傳資料中最新一期的月份（`YYYY-MM`）；空清單時為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// 月營收清單，依月份由新到舊。
    pub(crate) revenues: Vec<MonthlyRevenue>,
}

/// 單期財務報表（獲利能力與每股數據）。
///
/// `quarter` 依 §3.5 對映：資料庫以空字串代表年度資料，API 契約統一輸出
/// `A`；季度資料維持 `Q1`–`Q4`。百分比欄位（毛利率等）在資料庫已是
/// 百分比數值，不再另行換算。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct FinancialStatement {
    /// 年度（西元）。
    pub(crate) year: i64,
    /// 期間標記：`A`（年度）或 `Q1`–`Q4`。
    pub(crate) quarter: String,
    /// 營業毛利率（%）；DB 欄位 `gross_profit`。
    pub(crate) gross_profit_margin: Option<f64>,
    /// 營業利益率（%）。
    pub(crate) operating_profit_margin: Option<f64>,
    /// 稅前淨利率（%）；DB 欄位 `pre_tax_income`。
    pub(crate) pre_tax_income_margin: Option<f64>,
    /// 稅後淨利率（%）；DB 欄位 `net_income`。
    pub(crate) net_income_margin: Option<f64>,
    /// 每股淨值（元）。
    pub(crate) net_asset_value_per_share: Option<f64>,
    /// 每股營收（元）。
    pub(crate) sales_per_share: Option<f64>,
    /// 每股稅後盈餘 EPS（元）。
    pub(crate) earnings_per_share: Option<f64>,
    /// 每股稅前淨利（元）；DB 欄位 `profit_before_tax`。
    pub(crate) profit_before_tax_per_share: Option<f64>,
    /// 股東權益報酬率 ROE（%）。
    pub(crate) return_on_equity: Option<f64>,
    /// 資產報酬率 ROA（%）。
    pub(crate) return_on_assets: Option<f64>,
    /// 最後更新時間，UTC ISO 8601；DB 欄位 `updated_time`。
    pub(crate) updated_at: Option<String>,
}

/// 財報歷史的成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct FinancialStatementHistoryResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 實際回傳資料中最新一期的期間標記（如 `2026-Q1`、`2025-A`）；空清單時為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// 財報清單，依 §3.4 期間順序由新到舊。
    pub(crate) statements: Vec<FinancialStatement>,
}

/// 單筆股利發放資料。
///
/// 年度欄位語意（§4.3 對照表）：`paid_year` 是「發放年度」（DB `year`），
/// `dividend_year` 是「股利所屬年度」（DB `year_of_dividend`）；兩者常差
/// 一年，年份篩選一律依 `dividend_year`。日期欄位在資料庫是字串，只有可
/// 解析為合法 `YYYY-MM-DD` 的值才輸出，`-`、`尚未公布` 等標記一律 `null`。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct Dividend {
    /// 發放年度（西元）；DB 欄位 `year`。
    pub(crate) paid_year: i32,
    /// 股利所屬年度（西元）；DB 欄位 `year_of_dividend`。
    pub(crate) dividend_year: i32,
    /// 期間標記：`A`（年度）、`H1`／`H2`（半年度）或 `Q1`–`Q4`。
    pub(crate) quarter: String,
    /// 現金股利合計（元）。
    pub(crate) cash_dividend: Option<f64>,
    /// 股票股利合計（元）。
    pub(crate) stock_dividend: Option<f64>,
    /// 股利合計（元）；DB 欄位 `sum`。
    pub(crate) total_dividend: Option<f64>,
    /// 盈餘配息（元）。
    pub(crate) earnings_cash_dividend: Option<f64>,
    /// 公積配息（元）。
    pub(crate) capital_reserve_cash_dividend: Option<f64>,
    /// 盈餘配股（元）。
    pub(crate) earnings_stock_dividend: Option<f64>,
    /// 公積配股（元）。
    pub(crate) capital_reserve_stock_dividend: Option<f64>,
    /// 盈餘分配率＿配息（%）；DB 欄位 `payout_ratio_cash`。
    pub(crate) cash_payout_ratio: Option<f64>,
    /// 盈餘分配率＿配股（%）；DB 欄位 `payout_ratio_stock`。
    pub(crate) stock_payout_ratio: Option<f64>,
    /// 盈餘分配率合計（%）；DB 欄位 `payout_ratio`。
    pub(crate) total_payout_ratio: Option<f64>,
    /// 除息日；DB 欄位 `"ex-dividend_date1"`，無效標記為 `null`。
    pub(crate) ex_dividend_date: Option<String>,
    /// 除權日；DB 欄位 `"ex-dividend_date2"`，無效標記為 `null`。
    pub(crate) ex_rights_date: Option<String>,
    /// 現金股利發放日；DB 欄位 `payable_date1`，無效標記為 `null`。
    pub(crate) cash_payable_date: Option<String>,
    /// 股票股利發放日；DB 欄位 `payable_date2`，無效標記為 `null`。
    pub(crate) stock_payable_date: Option<String>,
    /// 最後更新時間，UTC ISO 8601；DB 欄位 `updated_time`。
    pub(crate) updated_at: Option<String>,
}

/// 股利歷史的成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct DividendHistoryResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 實際回傳資料中最新一期的期間標記（如 `2025-A`、`2025-Q4`）；空清單時為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// 股利清單，依 §3.4 期間順序由新到舊。
    pub(crate) dividends: Vec<Dividend>,
}

/// 單一交易日的個股估值計算結果。
///
/// 所有價格皆是 `estimate` 已完成的歷史模型計算值，不是目標價或買賣建議；
/// `valuation_band` 僅描述收盤價落在加權便宜／合理／昂貴價的哪一段。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct StockValuation {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 估值資料日期，格式 `YYYY-MM-DD`。
    pub(crate) date: String,
    /// 當日收盤價。
    pub(crate) closing_price: Option<f64>,
    /// 收盤價相對加權便宜價的百分比。
    pub(crate) percentage: Option<f64>,
    /// 參與模型計算的歷史年度數。
    pub(crate) year_count: i32,
    /// 加權便宜價。
    pub(crate) cheap: Option<f64>,
    /// 加權合理價。
    pub(crate) fair: Option<f64>,
    /// 加權昂貴價。
    pub(crate) expensive: Option<f64>,
    /// 歷史價格法便宜價。
    pub(crate) price_cheap: Option<f64>,
    /// 歷史價格法合理價。
    pub(crate) price_fair: Option<f64>,
    /// 歷史價格法昂貴價。
    pub(crate) price_expensive: Option<f64>,
    /// 股利法便宜價。
    pub(crate) dividend_cheap: Option<f64>,
    /// 股利法合理價。
    pub(crate) dividend_fair: Option<f64>,
    /// 股利法昂貴價。
    pub(crate) dividend_expensive: Option<f64>,
    /// EPS 法便宜價。
    pub(crate) eps_cheap: Option<f64>,
    /// EPS 法合理價。
    pub(crate) eps_fair: Option<f64>,
    /// EPS 法昂貴價。
    pub(crate) eps_expensive: Option<f64>,
    /// PBR 法便宜價。
    pub(crate) pbr_cheap: Option<f64>,
    /// PBR 法合理價。
    pub(crate) pbr_fair: Option<f64>,
    /// PBR 法昂貴價。
    pub(crate) pbr_expensive: Option<f64>,
    /// PER 法便宜價。
    pub(crate) per_cheap: Option<f64>,
    /// PER 法合理價。
    pub(crate) per_fair: Option<f64>,
    /// PER 法昂貴價。
    pub(crate) per_expensive: Option<f64>,
    /// 收盤價相對加權估值區間的固定分類。
    pub(crate) valuation_band: String,
}

/// 個股估值成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct StockValuationResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 實際回傳估值的日期；無資料時為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// 最近有效估值；31 天視窗內無資料時為 `null`。
    pub(crate) valuation: Option<StockValuation>,
}

/// 月營收歷史 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct RevenueHistoryParams {
    /// 起始月份，格式 YYYY-MM。
    pub(crate) from: Option<String>,
    /// 結束月份，格式 YYYY-MM。
    pub(crate) to: Option<String>,
    /// 最多回傳筆數，預設 24，範圍 1–120。
    #[param(minimum = 1, maximum = 120, default = 24)]
    pub(crate) limit: Option<u16>,
}

/// 財報歷史 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct StatementHistoryParams {
    /// 期間類型：`quarterly`（預設）、`annual` 或 `all`。
    #[param(value_type = StatementPeriodTypeValue, inline, default = "quarterly")]
    pub(crate) period_type: Option<String>,
    /// 最多回傳筆數，預設 12，範圍 1–40。
    #[param(minimum = 1, maximum = 40, default = 12)]
    pub(crate) limit: Option<u16>,
}

/// 股利歷史 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct DividendHistoryParams {
    /// 起始年度（股利所屬年度，西元）。
    #[param(minimum = 1990)]
    pub(crate) from_year: Option<i32>,
    /// 結束年度（股利所屬年度，西元）。
    #[param(minimum = 1990)]
    pub(crate) to_year: Option<i32>,
    /// 最多回傳筆數，預設 20，範圍 1–80。
    #[param(minimum = 1, maximum = 80, default = 20)]
    pub(crate) limit: Option<u16>,
}

/// 個股估值 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct ValuationParams {
    /// 查詢截止日，格式 `YYYY-MM-DD`；未提供時取最新資料。
    pub(crate) date: Option<String>,
}
