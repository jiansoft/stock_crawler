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

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    /// 工廠方法必須原樣保留各欄位，欄位順序寫錯會讓兩個 i64 序號互換而不易察覺。
    #[test]
    fn new_keeps_all_fields() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
        let rank = YieldRank::new(date, "2330".to_string(), 101, 202, dec!(2.35));

        assert_eq!(rank.date, date);
        assert_eq!(rank.security_code, "2330");
        assert_eq!(rank.daily_quotes_serial, 101);
        assert_eq!(rank.dividend_serial, 202);
        assert_eq!(rank.r#yield, dec!(2.35));
    }

    /// 同值實體必須相等（PartialEq 供去重與測試斷言使用）。
    #[test]
    fn equality_compares_by_value() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
        let one = YieldRank::new(date, "2330".to_string(), 101, 202, dec!(2.35));
        let another = YieldRank::new(date, "2330".to_string(), 101, 202, dec!(2.35));
        let different_yield = YieldRank::new(date, "2330".to_string(), 101, 202, dec!(2.36));

        assert_eq!(one, another);
        assert_ne!(one, different_yield);
    }
}
