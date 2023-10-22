use core::result::Result::Ok;

use anyhow::*;
use chrono::{DateTime, Duration, Local, NaiveDate};
use rust_decimal::Decimal;
use sqlx::{postgres::PgQueryResult, Row};

use crate::internal::{
    database, database::table::daily_quote::extension::MonthlyStockPriceSummary, util::datetime,
    StockExchange,
};

pub(crate) mod extension;

#[derive(sqlx::Type, sqlx::FromRow, Default, Debug, Clone)]
/// 每日股票報價數據
pub struct DailyQuote {
    pub maximum_price_in_year_date_on: NaiveDate,
    pub minimum_price_in_year_date_on: NaiveDate,
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
    /// 股價淨值比=每股股價 ÷ 每股淨值
    pub price_to_book_ratio: Decimal,
    pub security_code: String,
    pub serial: i64,
    pub year: i32,
    pub month: i32,
    pub day: i32,
}

impl DailyQuote {
    pub fn new(security_code: String) -> Self {
        DailyQuote {
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
        sqlx::query(sql)
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
            .bind(self.month)
            .bind(self.day)
            .execute(database::get_connection())
            .await
            .context(format!(
                "Failed to DailyQuote::upsert({:#?}) from database",
                self
            ))
    }

    /// 依指定日期取得收盤資料的均線
    pub async fn fill_moving_average(&mut self) -> Result<()> {
        let year_ago = self.date - Duration::days(400);
        let sql = r#"
WITH
cte AS (
    SELECT "Date","HighestPrice","LowestPrice","ClosingPrice"
    FROM "DailyQuotes"
    WHERE "SecurityCode" = $1 AND "Date" <= $2 AND "Date" >= $3
    ORDER BY "Date" DESC
	LIMIT 240
)
SELECT
(SELECT CASE WHEN COUNT(*) = 5   THEN round(COALESCE(AVG("ClosingPrice"),0),2) ELSE 0 END FROM (SELECT "ClosingPrice" FROM cte LIMIT 5)   AS a) AS "MovingAverage5",
(SELECT CASE WHEN COUNT(*) = 10  THEN round(COALESCE(AVG("ClosingPrice"),0),2) ELSE 0 END FROM (SELECT "ClosingPrice" FROM cte LIMIT 10)  AS a) AS "MovingAverage10",
(SELECT CASE WHEN COUNT(*) = 20  THEN round(COALESCE(AVG("ClosingPrice"),0),2) ELSE 0 END FROM (SELECT "ClosingPrice" FROM cte LIMIT 20)  AS a) AS "MovingAverage20",
(SELECT CASE WHEN COUNT(*) = 60  THEN round(COALESCE(AVG("ClosingPrice"),0),2) ELSE 0 END FROM (SELECT "ClosingPrice" FROM cte LIMIT 60)  AS a) AS "MovingAverage60",
(SELECT CASE WHEN COUNT(*) = 120 THEN round(COALESCE(AVG("ClosingPrice"),0),2) ELSE 0 END FROM (SELECT "ClosingPrice" FROM cte LIMIT 120) AS a) AS "MovingAverage120",
(SELECT CASE WHEN COUNT(*) = 240 THEN round(COALESCE(AVG("ClosingPrice"),0),2) ELSE 0 END FROM (SELECT "ClosingPrice" FROM cte LIMIT 240) AS a) AS "MovingAverage240",
(SELECT round(max("HighestPrice"),2) FROM cte) AS "maximum_price_in_year",
(SELECT "Date" FROM cte order by "HighestPrice" desc limit 1) AS "maximum_price_in_year_date_on",
(SELECT round(min("LowestPrice"),2) FROM cte) AS "minimum_price_in_year",
(SELECT "Date" FROM cte order by "LowestPrice" limit 1) AS "minimum_price_in_year_date_on",
(SELECT round(avg("ClosingPrice"),2) FROM cte) AS "average_price_in_year"
        "#;
        sqlx::query(sql)
            .bind(&self.security_code)
            .bind(self.date)
            .bind(year_ago)
            .try_map(|row: sqlx::postgres::PgRow| {
                self.moving_average_5 = row.get("MovingAverage5");
                self.moving_average_10 = row.get("MovingAverage10");
                self.moving_average_20 = row.get("MovingAverage20");
                self.moving_average_60 = row.get("MovingAverage60");
                self.moving_average_120 = row.get("MovingAverage120");
                self.moving_average_240 = row.get("MovingAverage240");
                self.maximum_price_in_year = row.get("maximum_price_in_year");
                self.maximum_price_in_year_date_on = row.get("maximum_price_in_year_date_on");
                self.minimum_price_in_year = row.get("minimum_price_in_year");
                self.minimum_price_in_year_date_on = row.get("minimum_price_in_year_date_on");
                self.average_price_in_year = row.get("average_price_in_year");

                Ok(())
            })
            .fetch_one(database::get_connection())
            .await
            .context(format!(
                "Failed to fetch_moving_average(security_code:{},date:{}) from database",
                self.security_code, self.date
            ))
    }

    //更新均線值
    pub async fn update_moving_average(&self) -> Result<PgQueryResult> {
        let sql = r#"
UPDATE "DailyQuotes"
SET
    "MovingAverage5" = $2,
    "MovingAverage10" = $3,
    "MovingAverage20" = $4,
    "MovingAverage60" = $5,
    "MovingAverage120" = $6,
    "MovingAverage240" = $7,
    maximum_price_in_year = $8,
    minimum_price_in_year = $9,
    average_price_in_year = $10,
    maximum_price_in_year_date_on = $11,
    minimum_price_in_year_date_on = $12,
    "price-to-book_ratio" = $13
WHERE "Serial" = $1
"#;
        sqlx::query(sql)
            .bind(self.serial)
            .bind(self.moving_average_5)
            .bind(self.moving_average_10)
            .bind(self.moving_average_20)
            .bind(self.moving_average_60)
            .bind(self.moving_average_120)
            .bind(self.moving_average_240)
            .bind(self.maximum_price_in_year)
            .bind(self.minimum_price_in_year)
            .bind(self.average_price_in_year)
            .bind(self.maximum_price_in_year_date_on)
            .bind(self.minimum_price_in_year_date_on)
            .bind(self.price_to_book_ratio)
            .execute(database::get_connection())
            .await
            .context(format!(
                "Failed to update_moving_average({:#?}) from database",
                self
            ))
    }
}

pub trait FromWithExchange<T, U> {
    fn from_with_exchange(exchange: T, item: &U) -> Self;
}

//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);
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
        e.maximum_price_in_year_date_on = default_date.date_naive();
        e.minimum_price_in_year_date_on = default_date.date_naive();

        e
    }
}

/// 補上當日缺少的每日收盤數據
pub async fn makeup_for_the_lack_daily_quotes(date: NaiveDate) -> Result<PgQueryResult> {
    let date_str = date.format("%Y-%m-%d").to_string();
    let prev_date = (date - Duration::days(30)).format("%Y-%m-%d").to_string();

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

    sqlx::query(&sql)
        .execute(database::get_connection())
        .await
        .context(format!(
            "Failed to makeup_for_the_lack_daily_quotes from database\r\n{}",
            &sql
        ))
}

/// 依照指定的年月取得該股票其月份的最低、平均、最高價
pub async fn fetch_monthly_stock_price_summary(
    stock_symbol: &str,
    year: i32,
    month: i32,
) -> Result<MonthlyStockPriceSummary> {
    let sql = r#"
SELECT
    MIN("LowestPrice") as lowest_price,
    AVG("ClosingPrice") as avg_price,
    MAX("HighestPrice") as highest_price
FROM "DailyQuotes"
WHERE "SecurityCode" = $1 AND "year" = $2 AND "month" = $3
GROUP BY "SecurityCode", "year", "month";
"#;
    Ok(sqlx::query_as::<_, MonthlyStockPriceSummary>(sql)
        .bind(stock_symbol)
        .bind(year)
        .bind(month)
        .fetch_one(database::get_connection())
        .await?)
}

/// # fetch_count_by_date
///
/// Fetches the count of daily quotes for the specified date.
///
/// # Arguments
///
/// * `date` - The date to fetch the count for in format: YYYY-MM-DD.
///
/// # Returns
///
/// Returns `Result<i64, sqlx::Error>`. This means the function returns a Result,
/// which would be either an i64 integer (count of daily quotes for the given date),
/// or an `sqlx::Error` error type when an error is encountered.
///
/// # Example
///
/// Below is an example of how this function can be used:
///
/// ```
/// use chrono::NaiveDate;
///
/// #[tokio::main]
/// async fn main() {
///     let date = NaiveDate::from_ymd(2022, 2, 21);
///     match fetch_count_by_date(date).await {
///         Ok(count) => println!("Count of daily quotes: {}", count),
///         Err(err) => println!("An error occurred: {:?}", err),
///     }
/// }
/// ```
pub async fn fetch_count_by_date(date: NaiveDate) -> Result<i64> {
    let sql = r#"SELECT count(*) FROM "DailyQuotes" WHERE "Date" = $1"#;
    let row: (i64,) = sqlx::query_as(sql)
        .bind(date)
        .fetch_one(database::get_connection())
        .await?;
    Ok(row.0)
}

pub async fn fetch_daily_quotes_by_date(date: NaiveDate) -> Result<Vec<DailyQuote>> {
    let sql = r#"
    SELECT
        "Serial",
        "Date",
        "SecurityCode",
        "TradingVolume",
        "Transaction",
        "TradeValue",
        "OpeningPrice",
        "HighestPrice",
        "LowestPrice",
        "ClosingPrice",
        "ChangeRange",
        "Change",
        "LastBestBidPrice",
        "LastBestBidVolume",
        "LastBestAskPrice",
        "LastBestAskVolume",
        "PriceEarningRatio",
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
        "price-to-book_ratio",
        year,
        month,
        day
    FROM "DailyQuotes"
    WHERE "Date" = $1"#;
    sqlx::query(sql)
        .bind(date)
        .try_map(|row: sqlx::postgres::PgRow| {
            let dq = DailyQuote {
                maximum_price_in_year_date_on: row.get("maximum_price_in_year_date_on"),
                minimum_price_in_year_date_on: row.get("minimum_price_in_year_date_on"),
                date: row.get("Date"),
                create_time: row.try_get("CreateTime")?,
                record_time: row.try_get("RecordTime")?,
                price_earning_ratio: row.get("PriceEarningRatio"),
                moving_average_60: row.get("MovingAverage60"),
                closing_price: row.get("ClosingPrice"),
                change_range: row.get("ChangeRange"),
                change: row.get("Change"),
                last_best_bid_price: row.get("LastBestBidPrice"),
                last_best_bid_volume: row.get("LastBestBidVolume"),
                last_best_ask_price: row.get("LastBestAskPrice"),
                last_best_ask_volume: row.get("LastBestAskVolume"),
                moving_average_5: row.get("MovingAverage5"),
                moving_average_10: row.get("MovingAverage10"),
                moving_average_20: row.get("MovingAverage20"),
                lowest_price: row.get("LowestPrice"),
                moving_average_120: row.get("MovingAverage120"),
                moving_average_240: row.get("MovingAverage240"),
                maximum_price_in_year: row.get("maximum_price_in_year"),
                minimum_price_in_year: row.get("minimum_price_in_year"),
                average_price_in_year: row.get("average_price_in_year"),
                highest_price: row.get("HighestPrice"),
                opening_price: row.get("OpeningPrice"),
                trading_volume: row.get("TradingVolume"),
                trade_value: row.get("TradeValue"),
                transaction: row.get("Transaction"),
                price_to_book_ratio: row.get("price-to-book_ratio"),
                security_code: row.get("SecurityCode"),
                serial: row.get("Serial"),
                year: row.get("year"),
                month: row.get("month"),
                day: row.get("day"),
            };

            Ok(dq)
        })
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch_daily_quotes_by_date from database")
}

#[cfg(test)]
mod tests {
    use chrono::Datelike;

    use crate::internal::{cache::SHARE, logging};

    use super::*;

    #[tokio::test]
    async fn test_fetch_moving_average() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_moving_average".to_string());
        let date = NaiveDate::from_ymd_opt(2023, 8, 1);
        let mut dq = DailyQuote::new("2330".to_string());
        dq.date = date.unwrap();
        match dq.fill_moving_average().await {
            Ok(_) => {
                dbg!(&dq);
                logging::debug_file_async(format!("fetch_moving_average: {:#?}", dq));
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to fetch_moving_average because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 fetch_moving_average".to_string());
    }

    #[tokio::test]
    async fn test_fetch_daily_quotes_by_date() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_daily_quotes_by_date".to_string());
        let date = NaiveDate::from_ymd_opt(2023, 7, 31);
        match fetch_daily_quotes_by_date(date.unwrap()).await {
            Ok(dqs) => {
                logging::debug_file_async(format!("fetch_daily_quotes_by_date: {:#?}", dqs));
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to fetch_daily_quotes_by_date because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 fetch_daily_quotes_by_date".to_string());
    }

    #[tokio::test]
    async fn test_fetch_count_by_date() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_count_by_date".to_string());
        let date = NaiveDate::from_ymd_opt(2023, 7, 31);
        match fetch_count_by_date(date.unwrap()).await {
            Ok(count) => {
                logging::debug_file_async(format!("count_by_date: {:?}", count));
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to fetch_count_by_date because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 fetch_count_by_date".to_string());
    }

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
    async fn test_fetch_lowest_avg_highest_price() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_lowest_avg_highest_price".to_string());

        match fetch_monthly_stock_price_summary("2330", 2023, 4).await {
            Ok(cd) => {
                logging::debug_file_async(format!("stock: {:?}", cd));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        logging::debug_file_async("結束 fetch_lowest_avg_highest_price".to_string());
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

        let mut e = DailyQuote::from_with_exchange(StockExchange::TWSE, &data);
        e.date = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
        e.year = e.date.year();
        e.month = e.date.month() as i32;
        e.day = e.date.day() as i32;
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

        let mut e = DailyQuote::from_with_exchange(StockExchange::TPEx, &otc);
        e.date = NaiveDate::from_ymd_opt(2000, 1, 2).unwrap();
        e.year = e.date.year();
        e.month = e.date.month() as i32;
        e.day = e.date.day() as i32;
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
