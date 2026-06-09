use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;

/// 每日個股報價領域實體 (Aggregate Root)。
///
/// 封裝單一股票在特定交易日的開高低收、成交量、均線與年內統計等欄位。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyQuote {
    /// 主鍵序號
    pub serial: i64,
    /// 股票代碼
    pub stock_symbol: String,
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
    /// 最後揭示買價
    pub last_best_bid_price: Decimal,
    /// 最後揭示買量
    pub last_best_bid_volume: Decimal,
    /// 最後揭示賣價
    pub last_best_ask_price: Decimal,
    /// 最後揭示賣量
    pub last_best_ask_volume: Decimal,
    /// 本益比
    pub price_earning_ratio: Decimal,
    /// 股價淨值比
    pub price_to_book_ratio: Decimal,
    /// 5 日均線
    pub moving_average_5: Decimal,
    /// 10 日均線
    pub moving_average_10: Decimal,
    /// 20 日均線
    pub moving_average_20: Decimal,
    /// 60 日均線
    pub moving_average_60: Decimal,
    /// 120 日均線
    pub moving_average_120: Decimal,
    /// 240 日均線
    pub moving_average_240: Decimal,
    /// 年內最高價
    pub maximum_price_in_year: Decimal,
    /// 年內最低價
    pub minimum_price_in_year: Decimal,
    /// 年內平均收盤價
    pub average_price_in_year: Decimal,
    /// 年內最高價對應日期
    pub maximum_price_in_year_date_on: NaiveDate,
    /// 年內最低價對應日期
    pub minimum_price_in_year_date_on: NaiveDate,
    /// 年、月、日（查詢用冗餘欄位）
    pub year: i32,
    pub month: i32,
    pub day: i32,
    /// 建立時間
    pub create_time: DateTime<Local>,
    /// 最後更新時間
    pub record_time: DateTime<Local>,
}

impl Default for DailyQuote {
    fn default() -> Self {
        let ep = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        DailyQuote {
            serial: 0,
            stock_symbol: String::new(),
            date: ep,
            opening_price: Decimal::ZERO,
            highest_price: Decimal::ZERO,
            lowest_price: Decimal::ZERO,
            closing_price: Decimal::ZERO,
            change: Decimal::ZERO,
            change_range: Decimal::ZERO,
            trading_volume: Decimal::ZERO,
            trade_value: Decimal::ZERO,
            transaction: Decimal::ZERO,
            last_best_bid_price: Decimal::ZERO,
            last_best_bid_volume: Decimal::ZERO,
            last_best_ask_price: Decimal::ZERO,
            last_best_ask_volume: Decimal::ZERO,
            price_earning_ratio: Decimal::ZERO,
            price_to_book_ratio: Decimal::ZERO,
            moving_average_5: Decimal::ZERO,
            moving_average_10: Decimal::ZERO,
            moving_average_20: Decimal::ZERO,
            moving_average_60: Decimal::ZERO,
            moving_average_120: Decimal::ZERO,
            moving_average_240: Decimal::ZERO,
            maximum_price_in_year: Decimal::ZERO,
            minimum_price_in_year: Decimal::ZERO,
            average_price_in_year: Decimal::ZERO,
            maximum_price_in_year_date_on: ep,
            minimum_price_in_year_date_on: ep,
            year: 0,
            month: 0,
            day: 0,
            create_time: Local::now(),
            record_time: Local::now(),
        }
    }
}

/// 個股最新交易日報價領域實體。
///
/// 用於記錄與快速查詢個股的最新價格狀態。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LastDailyQuote {
    /// 報價日期
    pub date: NaiveDate,
    /// 股票代號
    pub stock_symbol: String,
    /// 收盤價
    pub closing_price: Decimal,
}

impl Default for LastDailyQuote {
    fn default() -> Self {
        LastDailyQuote {
            date: NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            stock_symbol: String::new(),
            closing_price: Decimal::ZERO,
        }
    }
}

/// 每日全市場估值與技術面分布統計領域實體。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyStockPriceStats {
    /// 統計日期
    pub date: NaiveDate,
    /// 市場類型（0: 全部、2: TWSE、4: TPEx）
    pub stock_exchange_market_id: i32,
    /// 股價位階統計
    pub undervalued: i32,
    pub fair_valued: i32,
    pub overvalued: i32,
    pub highly_overvalued: i32,
    /// 均線上下的股票家數
    pub below_5_day_moving_average: i32,
    pub above_5_day_moving_average: i32,
    pub below_20_day_moving_average: i32,
    pub above_20_day_moving_average: i32,
    pub below_60_day_moving_average: i32,
    pub above_60_day_moving_average: i32,
    pub below_120_day_moving_average: i32,
    pub above_120_day_moving_average: i32,
    pub below_240_day_moving_average: i32,
    pub above_240_day_moving_average: i32,
    /// 漲跌家數
    pub stocks_up: i32,
    pub stocks_down: i32,
    pub stocks_unchanged: i32,
    /// 建立與更新時間
    pub created_at: DateTime<Local>,
    pub updated_at: DateTime<Local>,
}

impl Default for DailyStockPriceStats {
    fn default() -> Self {
        DailyStockPriceStats {
            date: NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            stock_exchange_market_id: 0,
            undervalued: 0,
            fair_valued: 0,
            overvalued: 0,
            highly_overvalued: 0,
            below_5_day_moving_average: 0,
            above_5_day_moving_average: 0,
            below_20_day_moving_average: 0,
            above_20_day_moving_average: 0,
            below_60_day_moving_average: 0,
            above_60_day_moving_average: 0,
            below_120_day_moving_average: 0,
            above_120_day_moving_average: 0,
            below_240_day_moving_average: 0,
            above_240_day_moving_average: 0,
            stocks_up: 0,
            stocks_down: 0,
            stocks_unchanged: 0,
            created_at: Local::now(),
            updated_at: Local::now(),
        }
    }
}

impl DailyQuote {
    /// 計算漲跌價差與漲跌幅百分比。
    ///
    /// # 參數
    /// - `yesterday_close`: 昨收價。
    pub fn calculate_change_range(&mut self, yesterday_close: Decimal) {
        // 昨收價大於零才進行計算，避免除以零
        if yesterday_close > Decimal::ZERO {
            // 計算今日與昨日收盤價的價差
            self.change = self.closing_price - yesterday_close;
            // 價差比率轉換為百分比
            self.change_range = (self.change / yesterday_close) * Decimal::from(100);
        } else {
            // 若昨收價非法則清空價差與漲幅
            self.change = Decimal::ZERO;
            self.change_range = Decimal::ZERO;
        }
    }

    /// 判定今日股價是否達到漲跌停限制。
    ///
    /// 依台股現行制度，漲跌幅絕對值大於等於 9.9% 視為觸及限制。
    pub fn is_limit_move(&self) -> bool {
        // 絕對值大於等於 9.9
        self.change_range.abs() >= Decimal::new(99, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_calculate_change_range() {
        // 建立測試用日報價，利用結構體初始化設定收盤價
        let mut quote = DailyQuote {
            closing_price: dec!(110),
            ..Default::default()
        };

        // 昨收為 100
        quote.calculate_change_range(dec!(100));
        // 預期上漲 10 元
        assert_eq!(quote.change, dec!(10));
        // 預期漲幅 10%
        assert_eq!(quote.change_range, dec!(10));

        // 昨收為 0（非法）
        quote.calculate_change_range(dec!(0));
        // 預期清零
        assert_eq!(quote.change, dec!(0));
        assert_eq!(quote.change_range, dec!(0));
    }

    #[test]
    fn test_is_limit_move() {
        // 正常波動 5.5%
        let quote_normal = DailyQuote {
            change_range: dec!(5.5),
            ..Default::default()
        };
        assert!(!quote_normal.is_limit_move());

        // 漲停 9.95%
        let quote_up = DailyQuote {
            change_range: dec!(9.95),
            ..Default::default()
        };
        assert!(quote_up.is_limit_move());

        // 跌停 -10.0%
        let quote_down = DailyQuote {
            change_range: dec!(-10.0),
            ..Default::default()
        };
        assert!(quote_down.is_limit_move());
    }
}
