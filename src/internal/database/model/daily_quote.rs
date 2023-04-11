use chrono::{DateTime, Local, NaiveDate};

use rust_decimal::Decimal;

#[derive(Default, Debug)]
/// 每日股票報價數據
pub struct Entity {
    pub maximum_price_in_year_date_on: DateTime<Local>,
    pub minimum_price_in_year_date_on: DateTime<Local>,
    pub date: NaiveDate,
    pub create_time: DateTime<Local>,
    pub record_time: DateTime<Local>,
    /// 本益比
    pub price_earning_ratio: Decimal,
    pub moving_average_60: Decimal,
    /// 收盤價
    pub closing_price: Decimal,
    pub change_range: Decimal,
    /// 漲跌價差
    pub change: Decimal,
    /// 最後揭示買價
    pub last_best_bid_price: Decimal,
    /// 最後揭示買量
    pub last_best_bid_volume: Decimal,
    /// 最後揭示賣價
    pub last_best_ask_price: Decimal,
    /// 最後揭示賣量
    pub last_best_ask_volume: Decimal,
    pub moving_average_5: Decimal,
    pub moving_average_10: Decimal,
    pub moving_average_20: Decimal,
    /// 最低價
    pub lowest_price: Decimal,
    pub moving_average_120: Decimal,
    pub moving_average_240: Decimal,
    pub maximum_price_in_year: Decimal,
    pub minimum_price_in_year: Decimal,
    pub average_price_in_year: Decimal,
    /// 最高價
    pub highest_price: Decimal,
    /// 開盤價
    pub opening_price: Decimal,
    /// 成交股數
    pub trading_volume: Decimal,
    /// 成交金額
    pub trade_value: Decimal,
    ///  成交筆數
    pub transaction: Decimal,
    pub price_to_book_ratio: Decimal,
    pub security_code: String,
    pub serial: i64,
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Entity {
    pub fn new(security_code: String) -> Self {
        Entity {
            security_code,
            ..Default::default()
        }
    }

    pub async fn upsert(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
