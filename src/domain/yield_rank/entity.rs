use chrono::NaiveDate;
use rust_decimal::Decimal;

/// 代表個股殖利率排行的領域實體。
///
/// 記錄特定交易日中，個股對應的報價序號、最新股利序號以及計算出的殖利率。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YieldRank {
    /// 交易日期
    pub date: NaiveDate,
    /// 股票代號
    pub security_code: String,
    /// 每日個股報價序號 (對應 DailyQuotes.Serial)
    pub daily_quotes_serial: i64,
    /// 股利發放序號 (對應 dividend.serial)
    pub dividend_serial: i64,
    /// 殖利率（百分比）
    pub r#yield: Decimal,
}

impl YieldRank {
    /// 建立全新殖利率排行實體的工廠方法。
    ///
    /// # 參數
    /// * `date` - 交易日期
    /// * `security_code` - 股票代號
    /// * `daily_quotes_serial` - 個股報價序號
    /// * `dividend_serial` - 股利發放序號
    /// * `r#yield` - 殖利率百分比
    pub fn new(
        date: NaiveDate,
        security_code: String,
        daily_quotes_serial: i64,
        dividend_serial: i64,
        r#yield: Decimal,
    ) -> Self {
        Self {
            date,
            security_code,
            daily_quotes_serial,
            dividend_serial,
            r#yield,
        }
    }
}
