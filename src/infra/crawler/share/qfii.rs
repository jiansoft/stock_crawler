//! # 外資及陸資持股共用型別
//!
//! 定義 TWSE 與 TPEx 外資及陸資持股狀況採集共用的資料載體 (DTO)。

use rust_decimal::Decimal;

/// 外資及陸資持股狀況爬蟲載體 (DTO)。
///
/// 用於存取從 TWSE 或 TPEx 採集到的外資及陸資持股統計基本資料。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QfiiDto {
    /// 證券代號
    pub stock_symbol: String,
    /// 已發行股數
    pub issued_share: i64,
    /// 全體外資及陸資持有股數
    pub shares_held: i64,
    /// 全體外資及陸資持股比率
    pub share_holding_percentage: Decimal,
}
