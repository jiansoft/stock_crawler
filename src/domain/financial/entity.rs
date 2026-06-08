use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;

/// 財務報表領域實體。
///
/// 封裝公司在特定年度與季度的財務數據（如毛利率、營業利益率、EPS 等）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinancialStatement {
    /// 序號
    pub serial: i64,
    /// 股票代號
    pub security_code: String,
    /// 年度
    pub year: i64,
    /// 季度 (Q1, Q2, Q3, Q4)
    pub quarter: String,
    /// 營業毛利率 (%)
    pub gross_profit: Decimal,
    /// 營業利益率 (%)
    pub operating_profit_margin: Decimal,
    /// 稅前淨利率 (%)
    pub pre_tax_income: Decimal,
    /// 稅後淨利率 (%)
    pub net_income: Decimal,
    /// 每股淨值 (元)
    pub net_asset_value_per_share: Decimal,
    /// 每股營收 (元)
    pub sales_per_share: Decimal,
    /// 每股稅後淨利 (EPS, 元)
    pub earnings_per_share: Decimal,
    /// 每股稅前淨利 (元)
    pub profit_before_tax: Decimal,
    /// 股東權益報酬率 (ROE, %)
    pub return_on_equity: Decimal,
    /// 資產報酬率 (ROA, %)
    pub return_on_assets: Decimal,
    /// 建立時間
    pub created_time: DateTime<Local>,
    /// 最後更新時間
    pub updated_time: DateTime<Local>,
}

impl Default for FinancialStatement {
    fn default() -> Self {
        FinancialStatement {
            serial: 0,
            security_code: String::new(),
            year: 0,
            quarter: String::new(),
            gross_profit: Decimal::ZERO,
            operating_profit_margin: Decimal::ZERO,
            pre_tax_income: Decimal::ZERO,
            net_income: Decimal::ZERO,
            net_asset_value_per_share: Decimal::ZERO,
            sales_per_share: Decimal::ZERO,
            earnings_per_share: Decimal::ZERO,
            profit_before_tax: Decimal::ZERO,
            return_on_equity: Decimal::ZERO,
            return_on_assets: Decimal::ZERO,
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }
}

impl FinancialStatement {
    /// 建立指定股票代號的財報模型。
    pub fn new(security_code: String) -> Self {
        FinancialStatement {
            security_code,
            ..Default::default()
        }
    }
}

/// 月營收領域實體。
///
/// 封裝公司在特定月份的營收與價格區間數據。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthlyRevenue {
    /// 股票代號
    pub stock_symbol: String,
    /// 當月營收 (元)
    pub monthly: Decimal,
    /// 上月營收 (元)
    pub last_month: Decimal,
    /// 去年當月營收 (元)
    pub last_year_this_month: Decimal,
    /// 當月累計營收 (元)
    pub monthly_accumulated: Decimal,
    /// 去年累計營收 (元)
    pub last_year_monthly_accumulated: Decimal,
    /// 上月比較增減 (%)
    pub compared_with_last_month: Decimal,
    /// 去年同月增減 (%)
    pub compared_with_last_year_same_month: Decimal,
    /// 前期比較增減 (%)
    pub accumulated_compared_with_last_year: Decimal,
    /// 月均價 (元)
    pub avg_price: Decimal,
    /// 當月最低價 (元)
    pub lowest_price: Decimal,
    /// 當月最高價 (元)
    pub highest_price: Decimal,
    /// 營收代表之日期 (timestamp，以 Unix 時間戳記表示，通常對齊至當月 1 日)
    pub date: i64,
    /// 建立時間
    pub create_time: DateTime<Local>,
}

impl Default for MonthlyRevenue {
    fn default() -> Self {
        MonthlyRevenue {
            stock_symbol: String::new(),
            monthly: Decimal::ZERO,
            last_month: Decimal::ZERO,
            last_year_this_month: Decimal::ZERO,
            monthly_accumulated: Decimal::ZERO,
            last_year_monthly_accumulated: Decimal::ZERO,
            compared_with_last_month: Decimal::ZERO,
            compared_with_last_year_same_month: Decimal::ZERO,
            accumulated_compared_with_last_year: Decimal::ZERO,
            avg_price: Decimal::ZERO,
            lowest_price: Decimal::ZERO,
            highest_price: Decimal::ZERO,
            date: 0,
            create_time: Local::now(),
        }
    }
}

impl MonthlyRevenue {
    /// 建立月營收實體預設值。
    pub fn new() -> Self {
        Default::default()
    }
}

/// 個股估值領域實體。
///
/// 彙整價格區間、股利法、EPS 法、PBR 法與 PER 法等估值結果。
#[derive(Debug, Clone, PartialEq)]
pub struct PriceEstimate {
    /// 估值日期
    pub date: NaiveDate,
    /// 參考的最後一筆日報價日期
    pub last_daily_quote_date: String,
    /// 股票代號
    pub security_code: String,
    /// 股票名稱
    pub name: String,
    /// 當日收盤價 (元)
    pub closing_price: f64,
    /// 估值百分比 (收盤價相對便宜價)
    pub percentage: f64,
    /// 加權便宜價 (元)
    pub cheap: f64,
    /// 加權合理價 (元)
    pub fair: f64,
    /// 加權昂貴價 (元)
    pub expensive: f64,
    /// 價格法便宜價 (元)
    pub price_cheap: f64,
    /// 價格法合理價 (元)
    pub price_fair: f64,
    /// 價格法昂貴價 (元)
    pub price_expensive: f64,
    /// 股利法便宜價 (元)
    pub dividend_cheap: f64,
    /// 股利法合理價 (元)
    pub dividend_fair: f64,
    /// 股利法昂貴價 (元)
    pub dividend_expensive: f64,
    /// EPS 法便宜價 (元)
    pub eps_cheap: f64,
    /// EPS 法合理價 (元)
    pub eps_fair: f64,
    /// EPS 法昂貴價 (元)
    pub eps_expensive: f64,
    /// PBR 法便宜價 (元)
    pub pbr_cheap: f64,
    /// PBR 法合理價 (元)
    pub pbr_fair: f64,
    /// PBR 法昂貴價 (元)
    pub pbr_expensive: f64,
    /// 參與統計的年度數
    pub year_count: i32,
    /// 內部排序或索引欄位
    pub index: i32,
}

impl Default for PriceEstimate {
    fn default() -> Self {
        PriceEstimate {
            date: NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            last_daily_quote_date: String::new(),
            security_code: String::new(),
            name: String::new(),
            closing_price: 0.0,
            percentage: 0.0,
            cheap: 0.0,
            fair: 0.0,
            expensive: 0.0,
            price_cheap: 0.0,
            price_fair: 0.0,
            price_expensive: 0.0,
            dividend_cheap: 0.0,
            dividend_fair: 0.0,
            dividend_expensive: 0.0,
            eps_cheap: 0.0,
            eps_fair: 0.0,
            eps_expensive: 0.0,
            pbr_cheap: 0.0,
            pbr_fair: 0.0,
            pbr_expensive: 0.0,
            year_count: 0,
            index: 0,
        }
    }
}

impl PriceEstimate {
    /// 建立個股估值實體。
    pub fn new(security_code: String, date: NaiveDate) -> Self {
        PriceEstimate {
            security_code,
            date,
            ..Default::default()
        }
    }
}
