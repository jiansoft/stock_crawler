use crate::infra::crawler::share::DailyQuoteDto;
use chrono::NaiveDate;
use rust_decimal::Decimal;

/// 儲存每日收盤報價命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveDailyQuoteCommand {
    /// 股票代號
    pub symbol: String,
    /// 交易日期
    pub date: NaiveDate,
    /// 開盤價
    pub opening_price: Decimal,
    /// 最高價
    pub highest_price: Decimal,
    /// 最低價
    pub lowest_price: Decimal,
    /// 收盤價
    pub closing_price: Decimal,
    /// 漲跌價差
    pub change: Decimal,
    /// 漲跌幅（百分比）
    pub change_range: Decimal,
    /// 成交股數
    pub trading_volume: Decimal,
    /// 成交金額
    pub trade_value: Decimal,
    /// 成交筆數
    pub transaction: Decimal,
    /// 本益比
    pub price_earning_ratio: Decimal,
    /// 股價淨值比
    pub price_to_book_ratio: Decimal,
    /// 最後揭示買價
    pub last_best_bid_price: Decimal,
    /// 最後揭示買量
    pub last_best_bid_volume: Decimal,
    /// 最後揭示賣價
    pub last_best_ask_price: Decimal,
    /// 最後揭示賣量
    pub last_best_ask_volume: Decimal,
}

/// 每日收盤報價爬蟲資料防腐層轉譯器。
pub struct QuoteAclMapper;

impl QuoteAclMapper {
    /// 將爬蟲 DTO 轉譯成系統內部的 `SaveDailyQuoteCommand`。
    pub fn from_dto(dto: &DailyQuoteDto) -> SaveDailyQuoteCommand {
        SaveDailyQuoteCommand {
            symbol: dto.symbol.clone(),
            date: dto.date,
            opening_price: dto.opening_price,
            highest_price: dto.highest_price,
            lowest_price: dto.lowest_price,
            closing_price: dto.closing_price,
            change: dto.change,
            change_range: dto.change_range,
            trading_volume: dto.trading_volume,
            trade_value: dto.trade_value,
            transaction: dto.transaction,
            price_earning_ratio: dto.price_earning_ratio,
            price_to_book_ratio: dto.price_to_book_ratio,
            last_best_bid_price: dto.last_best_bid_price,
            last_best_bid_volume: dto.last_best_bid_volume,
            last_best_ask_price: dto.last_best_ask_price,
            last_best_ask_volume: dto.last_best_ask_volume,
        }
    }

    /// 將 `SaveDailyQuoteCommand` 轉譯為領域模型 `DailyQuote`。
    pub fn from_command(cmd: &SaveDailyQuoteCommand) -> crate::domain::quote::entity::DailyQuote {
        use chrono::{Datelike, Local, TimeZone};
        use rust_decimal::Decimal;
        // 台北時區 (UTC+8)
        let timezone = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
        let record_time = cmd
            .date
            .and_hms_opt(15, 0, 0)
            .and_then(|naive| timezone.from_local_datetime(&naive).single())
            .unwrap_or_else(|| Local::now().with_timezone(&timezone));

        crate::domain::quote::entity::DailyQuote {
            serial: 0,
            stock_symbol: cmd.symbol.clone(),
            date: cmd.date,
            opening_price: cmd.opening_price,
            highest_price: cmd.highest_price,
            lowest_price: cmd.lowest_price,
            closing_price: cmd.closing_price,
            change: cmd.change,
            change_range: cmd.change_range,
            trading_volume: cmd.trading_volume,
            trade_value: cmd.trade_value,
            transaction: cmd.transaction,
            last_best_bid_price: cmd.last_best_bid_price,
            last_best_bid_volume: cmd.last_best_bid_volume,
            last_best_ask_price: cmd.last_best_ask_price,
            last_best_ask_volume: cmd.last_best_ask_volume,
            price_earning_ratio: cmd.price_earning_ratio,
            price_to_book_ratio: cmd.price_to_book_ratio,
            moving_average_5: Decimal::ZERO,
            moving_average_10: Decimal::ZERO,
            moving_average_20: Decimal::ZERO,
            moving_average_60: Decimal::ZERO,
            moving_average_120: Decimal::ZERO,
            moving_average_240: Decimal::ZERO,
            maximum_price_in_year: Decimal::ZERO,
            minimum_price_in_year: Decimal::ZERO,
            average_price_in_year: Decimal::ZERO,
            maximum_price_in_year_date_on: NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            minimum_price_in_year_date_on: NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            year: cmd.date.year(),
            month: cmd.date.month() as i32,
            day: cmd.date.day() as i32,
            create_time: Local::now(),
            record_time: record_time.with_timezone(&Local),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_quote_acl_mapping() {
        let dto = DailyQuoteDto {
            symbol: "2330".to_string(),
            date: NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
            opening_price: dec!(900.0),
            highest_price: dec!(910.0),
            lowest_price: dec!(895.0),
            closing_price: dec!(905.0),
            change: dec!(5.0),
            change_range: dec!(0.55),
            trading_volume: dec!(10000),
            trade_value: dec!(9000000),
            transaction: dec!(500),
            price_earning_ratio: dec!(20.5),
            price_to_book_ratio: dec!(5.2),
            last_best_bid_price: dec!(904.0),
            last_best_bid_volume: dec!(10),
            last_best_ask_price: dec!(905.0),
            last_best_ask_volume: dec!(20),
        };

        let cmd = QuoteAclMapper::from_dto(&dto);
        assert_eq!(cmd.symbol, "2330");
        assert_eq!(cmd.closing_price, dec!(905.0));

        let entity = QuoteAclMapper::from_command(&cmd);
        assert_eq!(entity.stock_symbol, "2330");
        assert_eq!(entity.closing_price, dec!(905.0));
        assert_eq!(entity.date, NaiveDate::from_ymd_opt(2026, 6, 5).unwrap());
    }
}
