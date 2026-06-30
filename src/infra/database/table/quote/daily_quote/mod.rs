use std::fmt::Write;

use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;

use crate::{
    core::declare::StockExchange,
    core::util::{datetime, map::Keyable},
    infra::database::CopyIn,
};

pub(crate) mod extension;
/// `DailyQuote` 的資料庫寫入／更新操作子模組。
mod mutation;
/// `DailyQuote` 的資料庫查詢操作子模組。
mod query;

pub use query::{
    fetch_count_by_date, fetch_daily_quotes_by_date, fetch_monthly_stock_price_summary,
    makeup_for_the_lack_daily_quotes,
};

#[derive(sqlx::Type, sqlx::FromRow, Default, Debug, Clone)]
/// 每日股票報價資料模型。
///
/// 對應資料表為 `DailyQuotes`，包含開高低收、成交量、
/// 均線、年內統計等欄位。  
/// 目前同時保留 `security_code` 與 `stock_symbol`，
/// 以兼容舊資料欄位與新欄位。
pub struct DailyQuote {
    /// 年內最高價對應日期。
    pub maximum_price_in_year_date_on: NaiveDate,
    /// 年內最低價對應日期。
    pub minimum_price_in_year_date_on: NaiveDate,
    /// 交易日期。
    pub date: NaiveDate,
    /// 建立時間。
    pub create_time: DateTime<Local>,
    /// 最後更新時間（資料寫入時間）。
    pub record_time: DateTime<Local>,
    /// 本益比
    pub price_earning_ratio: Decimal,
    /// 60 日均線。
    pub moving_average_60: Decimal,
    /// 收盤價
    pub closing_price: Decimal,
    /// 漲跌幅（百分比）。
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
    /// 5 日均線。
    pub moving_average_5: Decimal,
    /// 10 日均線。
    pub moving_average_10: Decimal,
    /// 20 日均線。
    pub moving_average_20: Decimal,
    /// 最低價
    pub lowest_price: Decimal,
    /// 120 日均線。
    pub moving_average_120: Decimal,
    /// 240 日均線。
    pub moving_average_240: Decimal,
    /// 年內最高價。
    pub maximum_price_in_year: Decimal,
    /// 年內最低價。
    pub minimum_price_in_year: Decimal,
    /// 年內平均收盤價。
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
    /// 股價淨值比=每股股價 ÷ 每股淨值
    pub price_to_book_ratio: Decimal,
    /// 股票代碼
    pub stock_symbol: String,
    /// 主鍵序號。
    pub serial: i64,
    /// 日期年份（冗餘欄位，利於查詢）。
    pub year: i32,
    /// 日期月份（冗餘欄位，利於查詢）。
    pub month: i32,
    /// 日期日（冗餘欄位，利於查詢）。
    pub day: i32,
}

/// 批次匯入 `DailyQuotes` 的 PostgreSQL `COPY` 指令。
pub const COPY_IN_QUERY: &str = r#"COPY "DailyQuotes"(
            maximum_price_in_year_date_on,
            minimum_price_in_year_date_on,
            "Date",
            "CreateTime",
            "RecordTime",
            "PriceEarningRatio",
            "MovingAverage60",
            "ClosingPrice",
            "ChangeRange",
            "Change",
            "LastBestBidPrice",
            "LastBestBidVolume",
            "LastBestAskPrice",
            "LastBestAskVolume",
            "MovingAverage5",
            "MovingAverage10",
            "MovingAverage20",
            "LowestPrice",
            "MovingAverage120",
            "MovingAverage240",
            maximum_price_in_year,
            minimum_price_in_year,
            average_price_in_year,
            "HighestPrice",
            "OpeningPrice",
            "TradingVolume",
            "TradeValue",
            "Transaction",
            "price-to-book_ratio",
            "stock_symbol",
            year,
            month,
            day) FROM STDIN WITH (FORMAT CSV)"#;

impl CopyIn for DailyQuote {
    fn to_csv(&self) -> String {
        self.to_csv()
    }
}

impl Keyable for DailyQuote {
    fn key(&self) -> String {
        format!("{}-{}", self.date.format("%Y%m%d"), self.stock_symbol)
    }

    fn key_with_prefix(&self) -> String {
        format!("DailyQuote:{}", self.key())
    }
}

impl DailyQuote {
    /// 建立 `DailyQuote` 預設實例，並同步初始化股票代碼欄位。
    pub fn new<S: Into<String>>(security_code: S) -> Self {
        let security_code = security_code.into();
        DailyQuote {
            stock_symbol: security_code,
            ..Default::default()
        }
    }

    /// 依欄位名稱映射，從單筆原始字串資料建立 `DailyQuote`。
    ///
    /// 適用於欄位順序可能變動的來源（例如 TWSE MI_INDEX）。
    pub fn from_with_map(item: &[String], map: &std::collections::HashMap<&str, usize>) -> Self {
        let code = map
            .get("證券代號")
            .and_then(|&i| item.get(i))
            .cloned()
            .unwrap_or_default();
        let mut e = DailyQuote::new(code);

        let parse_decimal = |key: &str| -> Decimal {
            map.get(key)
                .and_then(|&i| item.get(i))
                .map(|s| s.replace(',', ""))
                .and_then(|s| s.parse::<Decimal>().ok())
                .unwrap_or_default()
        };

        e.trading_volume = parse_decimal("成交股數");
        e.transaction = parse_decimal("成交筆數");
        e.trade_value = parse_decimal("成交金額");
        e.opening_price = parse_decimal("開盤價");
        e.highest_price = parse_decimal("最高價");
        e.lowest_price = parse_decimal("最低價");
        e.closing_price = parse_decimal("收盤價");
        e.change = parse_decimal("漲跌價差");
        e.last_best_bid_price = parse_decimal("最後揭示買價");
        e.last_best_bid_volume = parse_decimal("最後揭示買量");
        e.last_best_ask_price = parse_decimal("最後揭示賣價");
        e.last_best_ask_volume = parse_decimal("最後揭示賣量");
        e.price_earning_ratio = parse_decimal("本益比");

        // 處理漲跌符號
        if let Some(&i) = map.get("漲跌(+/-)")
            && let Some(sign) = item.get(i)
        {
            if sign.contains('-') || sign.contains('綠') {
                e.change = -e.change.abs();
            } else if sign.contains('+') || sign.contains('紅') {
                e.change = e.change.abs();
            }
        }

        e.create_time = Local::now();
        let default_date = datetime::parse_date("1970-01-01T00:00:00Z");
        e.maximum_price_in_year_date_on = default_date.date_naive();
        e.minimum_price_in_year_date_on = default_date.date_naive();

        e
    }

    /// 轉換為單行 CSV 字串，供 `COPY` 批次寫入使用。
    pub fn to_csv(&self) -> String {
        let mut csv_string = String::new();

        let _ = writeln!(
            csv_string,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            self.maximum_price_in_year_date_on,
            self.minimum_price_in_year_date_on,
            self.date,
            self.create_time,
            self.record_time,
            self.price_earning_ratio,
            self.moving_average_60,
            self.closing_price,
            self.change_range,
            self.change,
            self.last_best_bid_price,
            self.last_best_bid_volume,
            self.last_best_ask_price,
            self.last_best_ask_volume,
            self.moving_average_5,
            self.moving_average_10,
            self.moving_average_20,
            self.lowest_price,
            self.moving_average_120,
            self.moving_average_240,
            self.maximum_price_in_year,
            self.minimum_price_in_year,
            self.average_price_in_year,
            self.highest_price,
            self.opening_price,
            self.trading_volume,
            self.trade_value,
            self.transaction,
            self.price_to_book_ratio,
            self.stock_symbol,
            self.year,
            self.month,
            self.day,
        );

        csv_string
    }
}

/// 不同交易所來源轉換為統一資料模型的介面。
pub trait FromWithExchange<T, U> {
    /// 在給定交易所資訊的前提下，將來源資料轉成統一資料模型。
    fn from_with_exchange(exchange: T, item: &U) -> Self;
}

impl FromWithExchange<StockExchange, Vec<String>> for DailyQuote {
    fn from_with_exchange(exchange: StockExchange, item: &Vec<String>) -> Self {
        let mut e = DailyQuote::new(item[0].to_string());

        match exchange {
            StockExchange::TWSE => {
                let decimal_fields = [
                    (2, &mut e.trading_volume),
                    (3, &mut e.transaction),
                    (4, &mut e.trade_value),
                    (5, &mut e.opening_price),
                    (6, &mut e.highest_price),
                    (7, &mut e.lowest_price),
                    (8, &mut e.closing_price),
                    (10, &mut e.change),
                    (11, &mut e.last_best_bid_price),
                    (12, &mut e.last_best_bid_volume),
                    (13, &mut e.last_best_ask_price),
                    (14, &mut e.last_best_ask_volume),
                    (15, &mut e.price_earning_ratio),
                ];

                for (index, field) in decimal_fields {
                    let d = item.get(index).unwrap_or(&"".to_string()).replace(',', "");
                    *field = d.parse::<Decimal>().unwrap_or_default();
                }

                if let Some(change_str) = item.get(9)
                    && change_str.contains('-')
                {
                    e.change = -e.change;
                }
            }
            StockExchange::TPEx => {
                let decimal_fields = [
                    (7, &mut e.trading_volume),
                    (9, &mut e.transaction),
                    (8, &mut e.trade_value),
                    (4, &mut e.opening_price),
                    (5, &mut e.highest_price),
                    (6, &mut e.lowest_price),
                    (2, &mut e.closing_price),
                    (3, &mut e.change),
                    (10, &mut e.last_best_bid_price),
                    (11, &mut e.last_best_bid_volume),
                    (12, &mut e.last_best_ask_price),
                    (13, &mut e.last_best_ask_volume),
                ];

                for (index, field) in decimal_fields {
                    let d = item.get(index).unwrap_or(&"".to_string()).replace(',', "");
                    *field = d.parse::<Decimal>().unwrap_or_default();
                }
            }
            _ => {}
        }

        e.create_time = Local::now();
        let default_date = datetime::parse_date("1970-01-01T00:00:00Z");
        e.maximum_price_in_year_date_on = default_date.date_naive();
        e.minimum_price_in_year_date_on = default_date.date_naive();

        e
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rust_decimal_macros::dec;

    use super::*;

    fn default_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
    }

    #[test]
    fn new_sets_stock_symbol_and_default_key_fields() {
        let quote = DailyQuote::new("2330");

        assert_eq!(quote.stock_symbol, "2330");
        assert_eq!(quote.serial, 0);
        assert_eq!(quote.year, 0);
        assert_eq!(quote.month, 0);
        assert_eq!(quote.day, 0);
        assert_eq!(quote.closing_price, Decimal::ZERO);
    }

    #[test]
    fn key_and_key_with_prefix_use_date_and_symbol() {
        let mut quote = DailyQuote::new("2330");
        quote.date = NaiveDate::from_ymd_opt(2025, 2, 3).unwrap();

        assert_eq!(quote.key(), "20250203-2330");
        assert_eq!(quote.key_with_prefix(), "DailyQuote:20250203-2330");
    }

    #[test]
    fn from_with_map_parses_commas_and_negative_green_sign() {
        let item = vec![
            "2330".to_string(),
            "1,234,000".to_string(),
            "5,678".to_string(),
            "987,654,321".to_string(),
            "950.5".to_string(),
            "960.5".to_string(),
            "945.5".to_string(),
            "955.5".to_string(),
            "綠".to_string(),
            "12.5".to_string(),
            "955.0".to_string(),
            "100".to_string(),
            "956.0".to_string(),
            "200".to_string(),
            "20.5".to_string(),
        ];
        let map = HashMap::from([
            ("證券代號", 0),
            ("成交股數", 1),
            ("成交筆數", 2),
            ("成交金額", 3),
            ("開盤價", 4),
            ("最高價", 5),
            ("最低價", 6),
            ("收盤價", 7),
            ("漲跌(+/-)", 8),
            ("漲跌價差", 9),
            ("最後揭示買價", 10),
            ("最後揭示買量", 11),
            ("最後揭示賣價", 12),
            ("最後揭示賣量", 13),
            ("本益比", 14),
        ]);

        let quote = DailyQuote::from_with_map(&item, &map);

        assert_eq!(quote.stock_symbol, "2330");
        assert_eq!(quote.trading_volume, dec!(1234000));
        assert_eq!(quote.transaction, dec!(5678));
        assert_eq!(quote.trade_value, dec!(987654321));
        assert_eq!(quote.opening_price, dec!(950.5));
        assert_eq!(quote.highest_price, dec!(960.5));
        assert_eq!(quote.lowest_price, dec!(945.5));
        assert_eq!(quote.closing_price, dec!(955.5));
        assert_eq!(quote.change, dec!(-12.5));
        assert_eq!(quote.last_best_bid_price, dec!(955.0));
        assert_eq!(quote.last_best_bid_volume, dec!(100));
        assert_eq!(quote.last_best_ask_price, dec!(956.0));
        assert_eq!(quote.last_best_ask_volume, dec!(200));
        assert_eq!(quote.price_earning_ratio, dec!(20.5));
        assert_eq!(quote.maximum_price_in_year_date_on, default_date());
        assert_eq!(quote.minimum_price_in_year_date_on, default_date());
    }

    #[test]
    fn from_with_map_defaults_missing_or_invalid_fields() {
        let item = vec![
            "2330".to_string(),
            "not-a-number".to_string(),
            "+".to_string(),
            "bad-change".to_string(),
        ];
        let map = HashMap::from([
            ("證券代號", 0),
            ("成交股數", 1),
            ("漲跌(+/-)", 2),
            ("漲跌價差", 3),
            ("收盤價", 99),
        ]);

        let quote = DailyQuote::from_with_map(&item, &map);

        assert_eq!(quote.stock_symbol, "2330");
        assert_eq!(quote.trading_volume, Decimal::ZERO);
        assert_eq!(quote.change, Decimal::ZERO);
        assert_eq!(quote.closing_price, Decimal::ZERO);
    }

    #[test]
    fn from_with_exchange_twse_maps_signed_change_and_decimals() {
        let item = vec![
            "2330".to_string(),
            "台積電".to_string(),
            "1,234,000".to_string(),
            "5,678".to_string(),
            "987,654,321".to_string(),
            "950.5".to_string(),
            "960.5".to_string(),
            "945.5".to_string(),
            "955.5".to_string(),
            "<p style=color:green>-</p>".to_string(),
            "12.5".to_string(),
            "955.0".to_string(),
            "100".to_string(),
            "956.0".to_string(),
            "200".to_string(),
            "20.5".to_string(),
        ];

        let quote = DailyQuote::from_with_exchange(StockExchange::TWSE, &item);

        assert_eq!(quote.stock_symbol, "2330");
        assert_eq!(quote.trading_volume, dec!(1234000));
        assert_eq!(quote.transaction, dec!(5678));
        assert_eq!(quote.trade_value, dec!(987654321));
        assert_eq!(quote.opening_price, dec!(950.5));
        assert_eq!(quote.highest_price, dec!(960.5));
        assert_eq!(quote.lowest_price, dec!(945.5));
        assert_eq!(quote.closing_price, dec!(955.5));
        assert_eq!(quote.change, dec!(-12.5));
        assert_eq!(quote.price_earning_ratio, dec!(20.5));
        assert_eq!(quote.maximum_price_in_year_date_on, default_date());
        assert_eq!(quote.minimum_price_in_year_date_on, default_date());
    }

    #[test]
    fn from_with_exchange_tpex_maps_quote_fields() {
        let item = vec![
            "6488".to_string(),
            "環球晶".to_string(),
            "520.5".to_string(),
            "+3.5".to_string(),
            "518.0".to_string(),
            "525.0".to_string(),
            "515.0".to_string(),
            "78,000".to_string(),
            "40,599,000".to_string(),
            "63".to_string(),
            "520.0".to_string(),
            "2".to_string(),
            "521.0".to_string(),
            "3".to_string(),
        ];

        let quote = DailyQuote::from_with_exchange(StockExchange::TPEx, &item);

        assert_eq!(quote.stock_symbol, "6488");
        assert_eq!(quote.closing_price, dec!(520.5));
        assert_eq!(quote.change, dec!(3.5));
        assert_eq!(quote.opening_price, dec!(518.0));
        assert_eq!(quote.highest_price, dec!(525.0));
        assert_eq!(quote.lowest_price, dec!(515.0));
        assert_eq!(quote.trading_volume, dec!(78000));
        assert_eq!(quote.trade_value, dec!(40599000));
        assert_eq!(quote.transaction, dec!(63));
        assert_eq!(quote.last_best_bid_price, dec!(520.0));
        assert_eq!(quote.last_best_bid_volume, dec!(2));
        assert_eq!(quote.last_best_ask_price, dec!(521.0));
        assert_eq!(quote.last_best_ask_volume, dec!(3));
    }

    #[test]
    fn to_csv_contains_expected_columns_and_trailing_newline() {
        let mut quote = DailyQuote::new("2330");
        quote.date = NaiveDate::from_ymd_opt(2025, 2, 3).unwrap();
        quote.maximum_price_in_year_date_on = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
        quote.minimum_price_in_year_date_on = NaiveDate::from_ymd_opt(2025, 1, 2).unwrap();
        quote.closing_price = dec!(955.5);
        quote.change = dec!(-12.5);
        quote.year = 2025;
        quote.month = 2;
        quote.day = 3;

        let csv = quote.to_csv();

        assert!(csv.ends_with('\n'));
        assert!(csv.contains("2025-02-01,2025-01-02,2025-02-03"));
        assert!(csv.contains("955.5"));
        assert!(csv.contains("-12.5"));
        assert!(csv.contains(",2330,2025,2,3"));
        assert_eq!(csv.trim_end().split(',').count(), 33);
    }
}
