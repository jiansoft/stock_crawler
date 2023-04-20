use crate::{internal::database::DB, internal::util::datetime, internal::StockExchange};
use anyhow::*;
use chrono::{DateTime, Local, NaiveDate};
use core::result::Result::Ok;
use rust_decimal::Decimal;
use sqlx::postgres::PgQueryResult;

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

    pub async fn upsert(&self) -> Result<PgQueryResult> {
        let sql = r#"
       INSERT INTO "DailyQuotes" (
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
            "SecurityCode",
            year,
            month,
            day
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33)
        ON CONFLICT ("SecurityCode", "Date")
        DO UPDATE SET
            "RecordTime" = now(),
            "ClosingPrice" = excluded."ClosingPrice",
            "ChangeRange" = excluded."ChangeRange",
            "Change" = excluded. "Change",
            "LastBestBidPrice" = excluded. "LastBestBidPrice",
            "LastBestBidVolume" = excluded."LastBestBidVolume",
            "LastBestAskPrice" = excluded."LastBestAskPrice",
            "LastBestAskVolume" = excluded."LastBestAskVolume",
            "LowestPrice" = excluded."LowestPrice",
            "HighestPrice" = excluded."HighestPrice",
            "OpeningPrice" = excluded."OpeningPrice",
            "TradingVolume" = excluded."TradingVolume",
            "TradeValue" = excluded."TradeValue",
            "Transaction" = excluded."Transaction"
    "#;
        let result = sqlx::query(sql)
            .bind(self.maximum_price_in_year_date_on)
            .bind(self.minimum_price_in_year_date_on)
            .bind(self.date)
            .bind(self.create_time)
            .bind(self.record_time)
            .bind(self.price_earning_ratio)
            .bind(self.moving_average_60)
            .bind(self.closing_price)
            .bind(self.change_range)
            .bind(self.change)
            .bind(self.last_best_bid_price)
            .bind(self.last_best_bid_volume)
            .bind(self.last_best_ask_price)
            .bind(self.last_best_ask_volume)
            .bind(self.moving_average_5)
            .bind(self.moving_average_10)
            .bind(self.moving_average_20)
            .bind(self.lowest_price)
            .bind(self.moving_average_120)
            .bind(self.moving_average_240)
            .bind(self.maximum_price_in_year)
            .bind(self.minimum_price_in_year)
            .bind(self.average_price_in_year)
            .bind(self.highest_price)
            .bind(self.opening_price)
            .bind(self.trading_volume)
            .bind(self.trade_value)
            .bind(self.transaction)
            .bind(self.price_to_book_ratio)
            .bind(&self.security_code)
            .bind(self.year)
            .bind(self.month as i32)
            .bind(self.day as i32)
            .execute(&DB.pool)
            .await?;

        Ok(result)
    }
}

pub trait FromWithExchange<T, U> {
    fn from_with_exchange(exchange: T, item: &U) -> Self;
}

//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);
impl FromWithExchange<StockExchange, Vec<String>> for Entity {
    fn from_with_exchange(exchange: StockExchange, item: &Vec<String>) -> Self {
        let mut e = Entity::new(item[0].to_string());

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

                if let Some(change_str) = item.get(9) {
                    if change_str.contains('-') {
                        e.change = -e.change;
                    }
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
        }

        e.create_time = Local::now();
        let default_date = datetime::parse_date("1970-01-01T00:00:00Z");
        e.maximum_price_in_year_date_on = default_date;
        e.minimum_price_in_year_date_on = default_date;

        e
    }
}

/// 補上當日缺少的每日收盤數據
pub async fn makeup_for_the_lack_daily_quotes(date: NaiveDate) -> Result<PgQueryResult> {
    let date_str = date.format("%Y-%m-%d").to_string();
    let prev_date = (date - chrono::Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();

    let sql = format!(
        r#"
INSERT INTO "DailyQuotes" (
    "Date", "SecurityCode", "TradingVolume", "Transaction",
    "TradeValue", "OpeningPrice", "HighestPrice", "LowestPrice",
    "ClosingPrice", "ChangeRange", "Change", "LastBestBidPrice",
    "LastBestBidVolume", "LastBestAskPrice", "LastBestAskVolume",
    "PriceEarningRatio", "RecordTime", "CreateTime", "MovingAverage5",
    "MovingAverage10", "MovingAverage20", "MovingAverage60",
    "MovingAverage120", "MovingAverage240", maximum_price_in_year,
    minimum_price_in_year, average_price_in_year,
    maximum_price_in_year_date_on, minimum_price_in_year_date_on,
    "price-to-book_ratio"
)
SELECT '{0}' as "Date",
    "SecurityCode",
    0 as "TradingVolume",
    0 as "Transaction",
    0 as "TradeValue",
    "OpeningPrice",
    "HighestPrice",
    "LowestPrice",
    "ClosingPrice",
    0 as "ChangeRange",
    0 as "Change",
    0 as "LastBestBidPrice",
    0 as "LastBestBidVolume",
    0 as "LastBestAskPrice",
    0 as "LastBestAskVolume",
    0 as "PriceEarningRatio",
    "RecordTime",
    "CreateTime",
    "MovingAverage5",
    "MovingAverage10",
    "MovingAverage20",
    "MovingAverage60",
    "MovingAverage120",
    "MovingAverage240",
    maximum_price_in_year,
    minimum_price_in_year,
    average_price_in_year,
    maximum_price_in_year_date_on,
    minimum_price_in_year_date_on,
    "price-to-book_ratio"
FROM "DailyQuotes"
WHERE "Serial" IN
(
    SELECT MAX("Serial")
    FROM "DailyQuotes"
    WHERE "SecurityCode" IN
    (
        SELECT c.stock_symbol
        FROM stocks AS c
        WHERE stock_symbol NOT IN
        (
            SELECT "DailyQuotes"."SecurityCode"
            FROM "DailyQuotes"
            WHERE "Date" = '{0}'
        )
        AND c."SuspendListing" = false
    )
    AND "Date" < '{0}'
    AND "Date" > '{1}'
    GROUP BY "SecurityCode"
)"#,
        date_str, prev_date
    );

    Ok(sqlx::query(&sql).execute(&DB.pool).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::cache::SHARE;
    use crate::logging;
    use chrono::Datelike;

    #[tokio::test]
    async fn test_makeup_for_the_lack_daily_quotes() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        let now = Local::now().date_naive();

        logging::debug_file_async("開始 makeup_for_the_lack_daily_quotes".to_string());

        match makeup_for_the_lack_daily_quotes(now).await {
            Ok(result) => {
                logging::debug_file_async(format!("result:{:#?}", result));
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to makeup_for_the_lack_daily_quotes because:{:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 makeup_for_the_lack_daily_quotes".to_string());
    }

    #[tokio::test]
    async fn test_upsert() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 upsert".to_string());

        let data = vec![
            "79979".to_string(),
            "台泥".to_string(),
            "28,131,977".to_string(),
            "12,278".to_string(),
            "1,070,452,844".to_string(),
            "37.65".to_string(),
            "38.30".to_string(),
            "37.65".to_string(),
            "37.95".to_string(),
            "<p style= color:red>+</p>".to_string(),
            "0.40".to_string(),
            "37.95".to_string(),
            "139".to_string(),
            "38.00".to_string(),
            "309".to_string(),
            "51.28".to_string(),
        ];

        let mut e = Entity::from_with_exchange(StockExchange::TWSE, &data);
        e.date = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
        e.year = e.date.year();
        e.month = e.date.month();
        e.day = e.date.day();
        e.record_time = Local::now();
        e.create_time = Local::now();

        match e.upsert().await {
            Ok(_) => {
                /* logging::info_file_async(format!("word_id:{} e:{:#?}", word_id, &e));
                let _ = sqlx::query("delete from company_word where word_id = $1;")
                    .bind(word_id)
                    .execute(&DB.pool)
                    .await;*/
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to upsert because:{:?}", why));
            }
        }

        let otc = vec![
            "79979".to_string(),
            "茂生農經".to_string(),
            "46.55".to_string(),
            "+0.30".to_string(),
            "46.25".to_string(),
            "46.80".to_string(),
            "46.25".to_string(),
            "78,000".to_string(),
            "3,632,550".to_string(),
            "63".to_string(),
            "46.30".to_string(),
            "2".to_string(),
            "46.60".to_string(),
            "2".to_string(),
            "38,598,194".to_string(),
            "51.20".to_string(),
            "41.90".to_string(),
        ];

        let mut e = Entity::from_with_exchange(StockExchange::TPEx, &otc);
        e.date = NaiveDate::from_ymd_opt(2000, 1, 2).unwrap();
        e.year = e.date.year();
        e.month = e.date.month();
        e.day = e.date.day();
        e.record_time = Local::now();
        e.create_time = Local::now();

        match e.upsert().await {
            Ok(_) => {
                /* logging::info_file_async(format!("word_id:{} e:{:#?}", word_id, &e));
                let _ = sqlx::query("delete from company_word where word_id = $1;")
                    .bind(word_id)
                    .execute(&DB.pool)
                    .await;*/
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to upsert because:{:?}", why));
            }
        }
        logging::debug_file_async("結束 upsert".to_string());
    }
}
