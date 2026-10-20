//! 便宜、合理、昂貴價的估價實體。

use chrono::NaiveDate;

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
