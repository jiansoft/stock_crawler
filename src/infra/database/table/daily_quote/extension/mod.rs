use rust_decimal::Decimal;

/// 指定股票在單一月份的價格摘要（低/均/高）。
#[derive(sqlx::Type, sqlx::FromRow, Default, Debug)]
pub struct MonthlyStockPriceSummary {
    /// 最高價
    pub highest_price: Decimal,
    /// 最低價
    pub lowest_price: Decimal,
    /// 平均價
    pub avg_price: Decimal,
}
