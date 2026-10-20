//! 報價的領域實體與資料表模型之間的轉換（每日報價、最後交易日報價、歷史價格紀錄）。

use super::*;

impl From<DomainDailyQuote> for TableDailyQuote {
    fn from(domain: DomainDailyQuote) -> Self {
        // 將領域層的每日報價實體轉換為資料庫 Table 模型，完整對齊所有屬性
        TableDailyQuote {
            maximum_price_in_year_date_on: domain.maximum_price_in_year_date_on,
            minimum_price_in_year_date_on: domain.minimum_price_in_year_date_on,
            date: domain.date,
            create_time: domain.create_time,
            record_time: domain.record_time,
            price_earning_ratio: domain.price_earning_ratio,
            moving_average_60: domain.moving_average_60,
            closing_price: domain.closing_price,
            change_range: domain.change_range,
            change: domain.change,
            last_best_bid_price: domain.last_best_bid_price,
            last_best_bid_volume: domain.last_best_bid_volume,
            last_best_ask_price: domain.last_best_ask_price,
            last_best_ask_volume: domain.last_best_ask_volume,
            moving_average_5: domain.moving_average_5,
            moving_average_10: domain.moving_average_10,
            moving_average_20: domain.moving_average_20,
            lowest_price: domain.lowest_price,
            moving_average_120: domain.moving_average_120,
            moving_average_240: domain.moving_average_240,
            maximum_price_in_year: domain.maximum_price_in_year,
            minimum_price_in_year: domain.minimum_price_in_year,
            average_price_in_year: domain.average_price_in_year,
            highest_price: domain.highest_price,
            opening_price: domain.opening_price,
            trading_volume: domain.trading_volume,
            trade_value: domain.trade_value,
            transaction: domain.transaction,
            price_to_book_ratio: domain.price_to_book_ratio,
            stock_symbol: domain.stock_symbol,
            serial: domain.serial,
            year: domain.year,
            month: domain.month,
            day: domain.day,
        }
    }
}

impl From<TableDailyQuote> for DomainDailyQuote {
    fn from(table: TableDailyQuote) -> Self {
        // 將資料庫 Table 模型的每日報價轉換為領域層實體
        DomainDailyQuote {
            serial: table.serial,
            stock_symbol: table.stock_symbol,
            date: table.date,
            opening_price: table.opening_price,
            highest_price: table.highest_price,
            lowest_price: table.lowest_price,
            closing_price: table.closing_price,
            change: table.change,
            change_range: table.change_range,
            trading_volume: table.trading_volume,
            trade_value: table.trade_value,
            transaction: table.transaction,
            last_best_bid_price: table.last_best_bid_price,
            last_best_bid_volume: table.last_best_bid_volume,
            last_best_ask_price: table.last_best_ask_price,
            last_best_ask_volume: table.last_best_ask_volume,
            price_earning_ratio: table.price_earning_ratio,
            price_to_book_ratio: table.price_to_book_ratio,
            moving_average_5: table.moving_average_5,
            moving_average_10: table.moving_average_10,
            moving_average_20: table.moving_average_20,
            moving_average_60: table.moving_average_60,
            moving_average_120: table.moving_average_120,
            moving_average_240: table.moving_average_240,
            maximum_price_in_year: table.maximum_price_in_year,
            minimum_price_in_year: table.minimum_price_in_year,
            average_price_in_year: table.average_price_in_year,
            maximum_price_in_year_date_on: table.maximum_price_in_year_date_on,
            minimum_price_in_year_date_on: table.minimum_price_in_year_date_on,
            year: table.year,
            month: table.month,
            day: table.day,
            create_time: table.create_time,
            record_time: table.record_time,
        }
    }
}

impl From<DomainLastDailyQuote> for TableLastDailyQuotes {
    fn from(domain: DomainLastDailyQuote) -> Self {
        // 將領域層最新收盤價實體轉換為資料庫 Table 模型
        TableLastDailyQuotes {
            date: domain.date,
            closing_price: domain.closing_price,
            stock_symbol: domain.stock_symbol,
        }
    }
}

impl From<TableLastDailyQuotes> for DomainLastDailyQuote {
    fn from(table: TableLastDailyQuotes) -> Self {
        // 將資料庫 Table 模型最新收盤價轉換為領域層實體
        DomainLastDailyQuote {
            date: table.date,
            closing_price: table.closing_price,
            stock_symbol: table.stock_symbol,
        }
    }
}

impl From<DomainQuoteHistoryRecord> for TableQuoteHistoryRecord {
    fn from(domain: DomainQuoteHistoryRecord) -> Self {
        // 將領域層歷史極值紀錄實體轉換為資料庫 Table 模型
        TableQuoteHistoryRecord {
            maximum_price_date_on: domain.maximum_price_date_on,
            minimum_price_date_on: domain.minimum_price_date_on,
            maximum_price_to_book_ratio_date_on: domain.maximum_price_to_book_ratio_date_on,
            minimum_price_to_book_ratio_date_on: domain.minimum_price_to_book_ratio_date_on,
            security_code: domain.security_code,
            maximum_price: domain.maximum_price,
            minimum_price: domain.minimum_price,
            maximum_price_to_book_ratio: domain.maximum_price_to_book_ratio,
            minimum_price_to_book_ratio: domain.minimum_price_to_book_ratio,
        }
    }
}

impl From<TableQuoteHistoryRecord> for DomainQuoteHistoryRecord {
    fn from(table: TableQuoteHistoryRecord) -> Self {
        // 將資料庫 Table 模型歷史極值紀錄轉換為領域層實體
        DomainQuoteHistoryRecord {
            maximum_price_date_on: table.maximum_price_date_on,
            minimum_price_date_on: table.minimum_price_date_on,
            maximum_price_to_book_ratio_date_on: table.maximum_price_to_book_ratio_date_on,
            minimum_price_to_book_ratio_date_on: table.minimum_price_to_book_ratio_date_on,
            security_code: table.security_code,
            maximum_price: table.maximum_price,
            minimum_price: table.minimum_price,
            maximum_price_to_book_ratio: table.maximum_price_to_book_ratio,
            minimum_price_to_book_ratio: table.minimum_price_to_book_ratio,
        }
    }
}
