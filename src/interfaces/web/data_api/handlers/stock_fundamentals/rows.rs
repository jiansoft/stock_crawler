//! 基本面歷史 handlers 對應的資料庫列型別（月營收、財報、股利、估值）與轉成 API DTO 的規則。

use super::*;

/// 對應 `"Revenue"` 月營收列。
///
/// 欄位皆宣告為 `Option<Decimal>`：資料表雖為 NOT NULL，但 Option 解碼
/// 對非 NULL 值無額外成本，且與 `decimal_to_f64` 的簽名一致，未來 schema
/// 放寬也不會 panic。
#[derive(sqlx::FromRow)]
pub(super) struct RevenueRow {
    /// 營收月份（`YYYYMM` 整數）。
    pub(super) date: i64,
    /// 當月營收。
    monthly: Option<Decimal>,
    /// 上月營收。
    last_month: Option<Decimal>,
    /// 去年同月營收。
    last_year_this_month: Option<Decimal>,
    /// 本年度累計營收。
    monthly_accumulated: Option<Decimal>,
    /// 去年同期累計營收。
    last_year_monthly_accumulated: Option<Decimal>,
    /// 月增率。
    compared_with_last_month: Option<Decimal>,
    /// 年增率。
    compared_with_last_year_same_month: Option<Decimal>,
    /// 累計年增率。
    accumulated_compared_with_last_year: Option<Decimal>,
    /// 當月平均股價。
    avg_price: Option<Decimal>,
    /// 當月最低股價。
    lowest_price: Option<Decimal>,
    /// 當月最高股價。
    highest_price: Option<Decimal>,
}

impl RevenueRow {
    /// 將月營收列轉為 DTO，並讓每個 NUMERIC 欄位的轉換 log 帶股票代號。
    pub(super) fn into_dto(self, symbol: &str) -> MonthlyRevenue {
        // 固定欄位名由程式碼提供，不能由呼叫端注入；資料異常時仍只輸出 null。
        let convert = |value, field| analytical_decimal_to_f64(value, symbol, field);
        MonthlyRevenue {
            month: format_month(self.date),
            monthly_revenue: convert(self.monthly, "monthly_revenue"),
            last_month_revenue: convert(self.last_month, "last_month_revenue"),
            last_year_same_month_revenue: convert(
                self.last_year_this_month,
                "last_year_same_month_revenue",
            ),
            monthly_accumulated_revenue: convert(
                self.monthly_accumulated,
                "monthly_accumulated_revenue",
            ),
            last_year_monthly_accumulated_revenue: convert(
                self.last_year_monthly_accumulated,
                "last_year_monthly_accumulated_revenue",
            ),
            month_over_month_percent: convert(
                self.compared_with_last_month,
                "month_over_month_percent",
            ),
            year_over_year_percent: convert(
                self.compared_with_last_year_same_month,
                "year_over_year_percent",
            ),
            accumulated_year_over_year_percent: convert(
                self.accumulated_compared_with_last_year,
                "accumulated_year_over_year_percent",
            ),
            average_price: convert(self.avg_price, "average_price"),
            lowest_price: convert(self.lowest_price, "lowest_price"),
            highest_price: convert(self.highest_price, "highest_price"),
        }
    }
}

/// 對應 `financial_statement` 財報列。
#[derive(sqlx::FromRow)]
pub(super) struct StatementRow {
    /// 財報西元年度。
    pub(super) year: i64,
    /// 資料庫期間標記：空字串（年度）或 `Q1`–`Q4`。
    pub(super) quarter: String,
    /// 毛利率。
    gross_profit: Option<Decimal>,
    /// 營業利益率。
    operating_profit_margin: Option<Decimal>,
    /// 稅前淨利率。
    pre_tax_income: Option<Decimal>,
    /// 稅後淨利率。
    net_income: Option<Decimal>,
    /// 每股淨值。
    net_asset_value_per_share: Option<Decimal>,
    /// 每股營收。
    sales_per_share: Option<Decimal>,
    /// 每股盈餘。
    earnings_per_share: Option<Decimal>,
    /// 每股稅前淨利。
    profit_before_tax: Option<Decimal>,
    /// 股東權益報酬率。
    return_on_equity: Option<Decimal>,
    /// 資產報酬率。
    return_on_assets: Option<Decimal>,
    /// 資料庫最後更新時間。
    updated_time: Option<DateTime<Utc>>,
}
impl StatementRow {
    /// 將財報列轉成 API DTO，年度空字串映射為 `A` 並保留轉換錯誤上下文。
    pub(super) fn into_dto(self, symbol: &str) -> FinancialStatement {
        let convert = |value, field| analytical_decimal_to_f64(value, symbol, field);
        FinancialStatement {
            year: self.year,
            quarter: quarter_to_api(&self.quarter),
            gross_profit_margin: convert(self.gross_profit, "gross_profit_margin"),
            operating_profit_margin: convert(
                self.operating_profit_margin,
                "operating_profit_margin",
            ),
            pre_tax_income_margin: convert(self.pre_tax_income, "pre_tax_income_margin"),
            net_income_margin: convert(self.net_income, "net_income_margin"),
            net_asset_value_per_share: convert(
                self.net_asset_value_per_share,
                "net_asset_value_per_share",
            ),
            sales_per_share: convert(self.sales_per_share, "sales_per_share"),
            earnings_per_share: convert(self.earnings_per_share, "earnings_per_share"),
            profit_before_tax_per_share: convert(
                self.profit_before_tax,
                "profit_before_tax_per_share",
            ),
            return_on_equity: convert(self.return_on_equity, "return_on_equity"),
            return_on_assets: convert(self.return_on_assets, "return_on_assets"),
            updated_at: timestamp(self.updated_time),
        }
    }
}

/// 對應 `dividend` 股利列。
#[derive(sqlx::FromRow)]
pub(super) struct DividendRow {
    /// 發放年度（API `paid_year`）。
    year: i32,
    /// 股利所屬年度（API `dividend_year`）。
    pub(super) year_of_dividend: i32,
    /// 資料庫期間標記：空字串（年度）、`H1`／`H2` 或 `Q1`–`Q4`。
    pub(super) quarter: String,
    /// 現金股利合計。
    cash_dividend: Option<Decimal>,
    /// 股票股利合計。
    stock_dividend: Option<Decimal>,
    /// 現金與股票股利總和。
    sum: Option<Decimal>,
    /// 盈餘配息。
    earnings_cash_dividend: Option<Decimal>,
    /// 公積配息。
    capital_reserve_cash_dividend: Option<Decimal>,
    /// 盈餘配股。
    earnings_stock_dividend: Option<Decimal>,
    /// 公積配股。
    capital_reserve_stock_dividend: Option<Decimal>,
    /// 現金股利盈餘分配率。
    payout_ratio_cash: Option<Decimal>,
    /// 股票股利盈餘分配率。
    payout_ratio_stock: Option<Decimal>,
    /// 總盈餘分配率。
    payout_ratio: Option<Decimal>,
    /// 除息日（字串，可能是無效標記）。
    ex_dividend_date1: String,
    /// 除權日（字串，可能是無效標記）。
    ex_dividend_date2: String,
    /// 現金股利發放日（字串，可能是無效標記）。
    payable_date1: String,
    /// 股票股利發放日（字串，可能是無效標記）。
    payable_date2: String,
    /// 資料庫最後更新時間。
    updated_time: Option<DateTime<Utc>>,
}
impl DividendRow {
    /// 將股利列轉成 API DTO，清洗日期並以股票代號追蹤 NUMERIC 轉換失敗。
    pub(super) fn into_dto(self, symbol: &str) -> Dividend {
        let convert = |value, field| analytical_decimal_to_f64(value, symbol, field);
        Dividend {
            paid_year: self.year,
            dividend_year: self.year_of_dividend,
            quarter: quarter_to_api(&self.quarter),
            cash_dividend: convert(self.cash_dividend, "cash_dividend"),
            stock_dividend: convert(self.stock_dividend, "stock_dividend"),
            total_dividend: convert(self.sum, "total_dividend"),
            earnings_cash_dividend: convert(self.earnings_cash_dividend, "earnings_cash_dividend"),
            capital_reserve_cash_dividend: convert(
                self.capital_reserve_cash_dividend,
                "capital_reserve_cash_dividend",
            ),
            earnings_stock_dividend: convert(
                self.earnings_stock_dividend,
                "earnings_stock_dividend",
            ),
            capital_reserve_stock_dividend: convert(
                self.capital_reserve_stock_dividend,
                "capital_reserve_stock_dividend",
            ),
            cash_payout_ratio: convert(self.payout_ratio_cash, "cash_payout_ratio"),
            stock_payout_ratio: convert(self.payout_ratio_stock, "stock_payout_ratio"),
            total_payout_ratio: convert(self.payout_ratio, "total_payout_ratio"),
            ex_dividend_date: sanitize_date(&self.ex_dividend_date1),
            ex_rights_date: sanitize_date(&self.ex_dividend_date2),
            cash_payable_date: sanitize_date(&self.payable_date1),
            stock_payable_date: sanitize_date(&self.payable_date2),
            updated_at: timestamp(self.updated_time),
        }
    }
}

/// 對應 `estimate` 個股估值列。
#[derive(sqlx::FromRow)]
pub(super) struct ValuationRow {
    /// 股票代號。
    security_code: String,
    /// 估值日期。
    date: NaiveDate,
    /// 收盤價。
    closing_price: Decimal,
    /// 收盤價相對便宜價百分比。
    percentage: Decimal,
    /// 歷史樣本年度數。
    year_count: i32,
    /// 加權便宜價。
    cheap: Decimal,
    /// 加權合理價。
    fair: Decimal,
    /// 加權昂貴價。
    expensive: Decimal,
    /// 價格法便宜價。
    price_cheap: Decimal,
    /// 價格法合理價。
    price_fair: Decimal,
    /// 價格法昂貴價。
    price_expensive: Decimal,
    /// 股利法便宜價。
    dividend_cheap: Decimal,
    /// 股利法合理價。
    dividend_fair: Decimal,
    /// 股利法昂貴價。
    dividend_expensive: Decimal,
    /// EPS 法便宜價。
    eps_cheap: Decimal,
    /// EPS 法合理價。
    eps_fair: Decimal,
    /// EPS 法昂貴價。
    eps_expensive: Decimal,
    /// PBR 法便宜價。
    pbr_cheap: Decimal,
    /// PBR 法合理價。
    pbr_fair: Decimal,
    /// PBR 法昂貴價。
    pbr_expensive: Decimal,
    /// PER 法便宜價。
    per_cheap: Decimal,
    /// PER 法合理價。
    per_fair: Decimal,
    /// PER 法昂貴價。
    per_expensive: Decimal,
}

/// 將估值資料庫列轉成不暴露 Decimal 的 HTTP DTO。
impl From<ValuationRow> for StockValuation {
    fn from(row: ValuationRow) -> Self {
        let band = valuation_band(row.closing_price, row.cheap, row.fair, row.expensive);
        let symbol = row.security_code.clone();
        // 每個欄位都帶固定名稱進入轉換器，數值異常時能定位資料來源。
        let convert = |value, field| analytical_decimal_to_f64(Some(value), &symbol, field);
        Self {
            stock_symbol: row.security_code,
            date: row.date.to_string(),
            closing_price: convert(row.closing_price, "closing_price"),
            percentage: convert(row.percentage, "percentage"),
            year_count: row.year_count,
            cheap: convert(row.cheap, "cheap"),
            fair: convert(row.fair, "fair"),
            expensive: convert(row.expensive, "expensive"),
            price_cheap: convert(row.price_cheap, "price_cheap"),
            price_fair: convert(row.price_fair, "price_fair"),
            price_expensive: convert(row.price_expensive, "price_expensive"),
            dividend_cheap: convert(row.dividend_cheap, "dividend_cheap"),
            dividend_fair: convert(row.dividend_fair, "dividend_fair"),
            dividend_expensive: convert(row.dividend_expensive, "dividend_expensive"),
            eps_cheap: convert(row.eps_cheap, "eps_cheap"),
            eps_fair: convert(row.eps_fair, "eps_fair"),
            eps_expensive: convert(row.eps_expensive, "eps_expensive"),
            pbr_cheap: convert(row.pbr_cheap, "pbr_cheap"),
            pbr_fair: convert(row.pbr_fair, "pbr_fair"),
            pbr_expensive: convert(row.pbr_expensive, "pbr_expensive"),
            per_cheap: convert(row.per_cheap, "per_cheap"),
            per_fair: convert(row.per_fair, "per_fair"),
            per_expensive: convert(row.per_expensive, "per_expensive"),
            valuation_band: band.to_owned(),
        }
    }
}
