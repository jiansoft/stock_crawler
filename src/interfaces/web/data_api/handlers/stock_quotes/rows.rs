//! `/stocks/*` handlers 對應的資料庫列型別與轉成回應 DTO 的規則。

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;

use super::super::{decimal_to_f64, timestamp};
use crate::interfaces::web::data_api::dto::{
    DailyQuote, HistoricalQuote, QuoteHistoryRecord, Stock, StockProfile,
};

/// 對應 `stocks` 搜尋列。
#[derive(sqlx::FromRow)]
pub(super) struct StockRow {
    pub(super) stock_symbol: String,
    pub(super) security_code: String,
    pub(super) name: String,
    pub(super) stock_exchange_market_id: i32,
    pub(super) stock_industry_id: i32,
    pub(super) suspend_listing: bool,
}
impl From<StockRow> for Stock {
    fn from(row: StockRow) -> Self {
        Self {
            stock_symbol: row.stock_symbol,
            security_code: row.security_code,
            name: row.name,
            stock_exchange_market_id: row.stock_exchange_market_id,
            stock_industry_id: row.stock_industry_id,
            suspend_listing: row.suspend_listing,
        }
    }
}
/// 對應股票與可選最新日報價的 LEFT JOIN 列。
#[derive(sqlx::FromRow)]
pub(super) struct LatestQuoteRow {
    pub(super) stock_symbol: String,
    pub(super) security_code: String,
    pub(super) name: String,
    pub(super) stock_exchange_market_id: i32,
    pub(super) stock_industry_id: i32,
    pub(super) suspend_listing: bool,
    pub(super) date: Option<NaiveDate>,
    pub(super) opening_price: Option<Decimal>,
    pub(super) highest_price: Option<Decimal>,
    pub(super) lowest_price: Option<Decimal>,
    pub(super) closing_price: Option<Decimal>,
    pub(super) change: Option<Decimal>,
    pub(super) change_range: Option<Decimal>,
    pub(super) trading_volume: Option<Decimal>,
    pub(super) transaction: Option<Decimal>,
    pub(super) trade_value: Option<Decimal>,
    pub(super) moving_average_5: Option<Decimal>,
    pub(super) moving_average_10: Option<Decimal>,
    pub(super) moving_average_20: Option<Decimal>,
    pub(super) moving_average_60: Option<Decimal>,
    pub(super) moving_average_120: Option<Decimal>,
    pub(super) moving_average_240: Option<Decimal>,
    pub(super) price_earning_ratio: Option<Decimal>,
    pub(super) record_time: Option<DateTime<Utc>>,
    pub(super) updated_time: Option<DateTime<Utc>>,
}
impl LatestQuoteRow {
    /// 複製 LEFT JOIN 左側一定存在的股票主檔欄位。
    pub(super) fn stock(&self) -> Stock {
        Stock {
            stock_symbol: self.stock_symbol.clone(),
            security_code: self.security_code.clone(),
            name: self.name.clone(),
            stock_exchange_market_id: self.stock_exchange_market_id,
            stock_industry_id: self.stock_industry_id,
            suspend_listing: self.suspend_listing,
        }
    }
    /// 若 JOIN 到日報價則轉為 API DTO，否則維持 JSON null 語意。
    pub(super) fn quote(self) -> Option<DailyQuote> {
        self.date.map(|date| DailyQuote {
            date: date.to_string(),
            opening_price: decimal_to_f64(self.opening_price),
            highest_price: decimal_to_f64(self.highest_price),
            lowest_price: decimal_to_f64(self.lowest_price),
            closing_price: decimal_to_f64(self.closing_price),
            change: decimal_to_f64(self.change),
            change_range: decimal_to_f64(self.change_range),
            trading_volume: decimal_to_f64(self.trading_volume),
            transaction: decimal_to_f64(self.transaction),
            trade_value: decimal_to_f64(self.trade_value),
            moving_average_5: decimal_to_f64(self.moving_average_5),
            moving_average_10: decimal_to_f64(self.moving_average_10),
            moving_average_20: decimal_to_f64(self.moving_average_20),
            moving_average_60: decimal_to_f64(self.moving_average_60),
            moving_average_120: decimal_to_f64(self.moving_average_120),
            moving_average_240: decimal_to_f64(self.moving_average_240),
            price_earning_ratio: decimal_to_f64(self.price_earning_ratio),
            record_time: timestamp(self.record_time),
            updated_time: timestamp(self.updated_time),
        })
    }
}
/// 對應歷史日線列。
#[derive(sqlx::FromRow)]
pub(super) struct HistoricalQuoteRow {
    pub(super) date: NaiveDate,
    pub(super) opening_price: Option<Decimal>,
    pub(super) highest_price: Option<Decimal>,
    pub(super) lowest_price: Option<Decimal>,
    pub(super) closing_price: Option<Decimal>,
    pub(super) change: Option<Decimal>,
    pub(super) change_range: Option<Decimal>,
    pub(super) trading_volume: Option<Decimal>,
    pub(super) transaction: Option<Decimal>,
    pub(super) trade_value: Option<Decimal>,
    pub(super) moving_average_5: Option<Decimal>,
    pub(super) moving_average_10: Option<Decimal>,
    pub(super) moving_average_20: Option<Decimal>,
    pub(super) moving_average_60: Option<Decimal>,
    pub(super) price_earning_ratio: Option<Decimal>,
    pub(super) price_to_book_ratio: Option<Decimal>,
    pub(super) record_time: Option<DateTime<Utc>>,
}
impl From<HistoricalQuoteRow> for HistoricalQuote {
    fn from(row: HistoricalQuoteRow) -> Self {
        Self {
            date: row.date.to_string(),
            opening_price: decimal_to_f64(row.opening_price),
            highest_price: decimal_to_f64(row.highest_price),
            lowest_price: decimal_to_f64(row.lowest_price),
            closing_price: decimal_to_f64(row.closing_price),
            change: decimal_to_f64(row.change),
            change_range: decimal_to_f64(row.change_range),
            trading_volume: decimal_to_f64(row.trading_volume),
            transaction: decimal_to_f64(row.transaction),
            trade_value: decimal_to_f64(row.trade_value),
            moving_average_5: decimal_to_f64(row.moving_average_5),
            moving_average_10: decimal_to_f64(row.moving_average_10),
            moving_average_20: decimal_to_f64(row.moving_average_20),
            moving_average_60: decimal_to_f64(row.moving_average_60),
            price_earning_ratio: decimal_to_f64(row.price_earning_ratio),
            price_to_book_ratio: decimal_to_f64(row.price_to_book_ratio),
            record_time: timestamp(row.record_time),
            adjusted: None,
        }
    }
}
/// 對應 profile 三表 LEFT JOIN 列。
#[derive(sqlx::FromRow)]
pub(super) struct ProfileRow {
    #[sqlx(flatten)]
    pub(super) latest: LatestQuoteRow,
    pub(super) last_one_eps: Option<Decimal>,
    pub(super) last_four_eps: Option<Decimal>,
    pub(super) net_asset_value_per_share: Option<Decimal>,
    pub(super) return_on_equity: Option<Decimal>,
    pub(super) weight: Option<Decimal>,
    /// `stocks.issued_share` 是 bigint；宣告成 Decimal 會解碼失敗，整個 profile 回 500。
    pub(super) issued_share: Option<i64>,
    pub(super) maximum_price: Option<Decimal>,
    pub(super) maximum_price_date_on: Option<NaiveDate>,
    pub(super) minimum_price: Option<Decimal>,
    pub(super) minimum_price_date_on: Option<NaiveDate>,
    pub(super) maximum_price_to_book_ratio: Option<Decimal>,
    pub(super) maximum_price_to_book_ratio_date_on: Option<NaiveDate>,
    pub(super) minimum_price_to_book_ratio: Option<Decimal>,
    pub(super) minimum_price_to_book_ratio_date_on: Option<NaiveDate>,
}

impl From<ProfileRow> for StockProfile {
    fn from(row: ProfileRow) -> Self {
        let history = row
            .maximum_price_date_on
            .is_some()
            .then(|| QuoteHistoryRecord {
                maximum_price: decimal_to_f64(row.maximum_price),
                maximum_price_date_on: row.maximum_price_date_on.map(|date| date.to_string()),
                minimum_price: decimal_to_f64(row.minimum_price),
                minimum_price_date_on: row.minimum_price_date_on.map(|date| date.to_string()),
                maximum_price_to_book_ratio: decimal_to_f64(row.maximum_price_to_book_ratio),
                maximum_price_to_book_ratio_date_on: row
                    .maximum_price_to_book_ratio_date_on
                    .map(|date| date.to_string()),
                minimum_price_to_book_ratio: decimal_to_f64(row.minimum_price_to_book_ratio),
                minimum_price_to_book_ratio_date_on: row
                    .minimum_price_to_book_ratio_date_on
                    .map(|date| date.to_string()),
            });
        Self {
            stock: row.latest.stock(),
            quote: row.latest.quote(),
            last_one_eps: decimal_to_f64(row.last_one_eps),
            last_four_eps: decimal_to_f64(row.last_four_eps),
            net_asset_value_per_share: decimal_to_f64(row.net_asset_value_per_share),
            return_on_equity: decimal_to_f64(row.return_on_equity),
            weight: decimal_to_f64(row.weight),
            // 發行股數遠小於 2^53，轉 f64 不會失真。
            issued_share: row.issued_share.map(|shares| shares as f64),
            history,
        }
    }
}
