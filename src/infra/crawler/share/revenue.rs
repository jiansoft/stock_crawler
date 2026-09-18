//! # 月營收共用型別
//!
//! 定義月營收採集的資料載體 (DTO)，以及由來源原始字串欄位轉換成
//! 該載體的邏輯（欄位為空或無法解析時退回預設值）。

use rust_decimal::Decimal;

/// 營收資訊爬蟲載體 (DTO)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevenueDto {
    /// 股票代號
    pub stock_symbol: String,
    /// 當月營收
    pub monthly: Decimal,
    /// 上月營收
    pub last_month: Decimal,
    /// 去年當月營收
    pub last_year_this_month: Decimal,
    /// 當月累計營收
    pub monthly_accumulated: Decimal,
    /// 去年累計營收
    pub last_year_monthly_accumulated: Decimal,
    /// 上月比較增減(%)
    pub compared_with_last_month: Decimal,
    /// 去年同月增減(%)
    pub compared_with_last_year_same_month: Decimal,
    /// 前期比較增減(%)
    pub accumulated_compared_with_last_year: Decimal,
    /// 營收月份 (YYYYMM 格式整數，如 202605)
    pub date: i64,
}

impl From<Vec<String>> for RevenueDto {
    fn from(item: Vec<String>) -> Self {
        use std::str::FromStr;
        let stock_symbol = item[0].to_string();

        let monthly = {
            let s = item[2].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!("Failed to parse 'monthly'({}) field: {}", item[2], err);
                    Default::default()
                })
            }
        };
        let last_month = {
            let s = item[3].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!("Failed to parse 'last_month'({}) field: {}", item[3], err);
                    Default::default()
                })
            }
        };
        let last_year_this_month = {
            let s = item[4].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!(
                        "Failed to parse 'last_year_this_month'({}) field: {}",
                        item[4], err
                    );
                    Default::default()
                })
            }
        };
        let monthly_accumulated = {
            let s = item[7].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!(
                        "Failed to parse 'monthly_accumulated'({}) field: {}",
                        item[7], err
                    );
                    Default::default()
                })
            }
        };
        let last_year_monthly_accumulated = {
            let s = item[8].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!(
                        "Failed to parse 'last_year_monthly_accumulated'({}) field: {}",
                        item[8], err
                    );
                    Default::default()
                })
            }
        };
        let compared_with_last_month = {
            let s = item[5].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!(
                        "Failed to parse 'compared_with_last_month'({}) field: {}",
                        item[5], err
                    );
                    Default::default()
                })
            }
        };
        let compared_with_last_year_same_month = {
            let s = item[6].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!(
                        "Failed to parse 'compared_with_last_year_same_month'({}) field: {}",
                        item[6], err
                    );
                    Default::default()
                })
            }
        };
        let accumulated_compared_with_last_year = {
            let s = item[9].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!(
                        "Failed to parse 'accumulated_compared_with_last_year'({}) field: {}",
                        item[9], err
                    );
                    Default::default()
                })
            }
        };

        Self {
            stock_symbol,
            monthly,
            last_month,
            last_year_this_month,
            monthly_accumulated,
            last_year_monthly_accumulated,
            compared_with_last_month,
            compared_with_last_year_same_month,
            accumulated_compared_with_last_year,
            date: 0,
        }
    }
}
