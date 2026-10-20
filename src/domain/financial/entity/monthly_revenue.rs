//! 月營收實體。

use chrono::{DateTime, Local};
use rust_decimal::Decimal;

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
