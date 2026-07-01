//! `DailyQuote` 的資料庫查詢操作。
//!
//! 包含缺漏補齊、指定年月的價格統計、指定日期筆數，以及指定日期的全部報價讀取。

use anyhow::{Context, Result};
use chrono::{NaiveDate, TimeDelta};
use sqlx::{Row, postgres::PgQueryResult};

use crate::infra::database;

use super::DailyQuote;
use super::extension::MonthlyStockPriceSummary;

/// 補齊指定日期缺漏的每日收盤資料。
pub async fn makeup_for_the_lack_daily_quotes(date: NaiveDate) -> Result<PgQueryResult> {
    let prev_date = date - TimeDelta::try_days(30).unwrap();

    // 使用參數化查詢代替字串格式化，將 $1 與 $2 分別綁定 date 與 prev_date，以移除 AssertSqlSafe
    let sql = r#"
INSERT INTO "DailyQuotes" (
    "Date", "stock_symbol", "TradingVolume", "Transaction",
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
SELECT $1 as "Date",
    "stock_symbol",
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
    WHERE "stock_symbol" IN
    (
        SELECT c.stock_symbol
        FROM stocks AS c
        WHERE "stock_symbol" NOT IN
        (
            SELECT "DailyQuotes"."stock_symbol"
            FROM "DailyQuotes"
            WHERE "Date" = $1
        )
        AND c."SuspendListing" = false
    )
    AND "Date" < $1
    AND "Date" > $2
    GROUP BY "stock_symbol"
)"#;

    sqlx::query(sql)
        .bind(date)
        .bind(prev_date)
        .execute(database::get_connection())
        .await
        .context(format!(
            "Failed to makeup_for_the_lack_daily_quotes from database for date: {:?}",
            date
        ))
}

/// 取得指定股票在指定年月的最低、平均、最高收盤價統計。
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
WHERE "stock_symbol" = $1 AND "year" = $2 AND "month" = $3
GROUP BY "stock_symbol", "year", "month";
"#;
    Ok(sqlx::query_as::<_, MonthlyStockPriceSummary>(sql)
        .bind(stock_symbol)
        .bind(year)
        .bind(month)
        .fetch_one(database::get_connection())
        .await?)
}

/// 取得指定日期在 `DailyQuotes` 的資料筆數。
pub async fn fetch_count_by_date(date: NaiveDate) -> Result<i64> {
    let sql = r#"SELECT count(*) FROM "DailyQuotes" WHERE "Date" = $1"#;
    let row: (i64,) = sqlx::query_as(sql)
        .bind(date)
        .fetch_one(database::get_connection())
        .await?;
    Ok(row.0)
}

/// 讀取指定日期的所有日報價資料。
pub async fn fetch_daily_quotes_by_date(date: NaiveDate) -> Result<Vec<DailyQuote>> {
    let sql = r#"
    SELECT
        "Serial",
        "Date",
        "stock_symbol",
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
                stock_symbol: row.get("stock_symbol"),
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
    use chrono::Local;

    use crate::infra::cache::SHARE;

    use super::*;

    #[tokio::test]
    async fn test_fetch_daily_quotes_by_date() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 fetch_daily_quotes_by_date");
        let date = NaiveDate::from_ymd_opt(2023, 7, 31);
        match fetch_daily_quotes_by_date(date.unwrap()).await {
            Ok(dqs) => {
                tracing::debug!("fetch_daily_quotes_by_date: {:#?}", dqs);
            }
            Err(why) => {
                tracing::debug!("Failed to fetch_daily_quotes_by_date because {:?}", why);
            }
        }

        tracing::debug!("結束 fetch_daily_quotes_by_date");
    }

    #[tokio::test]
    async fn test_fetch_count_by_date() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 fetch_count_by_date");
        let date = NaiveDate::from_ymd_opt(2023, 7, 31);
        match fetch_count_by_date(date.unwrap()).await {
            Ok(count) => {
                tracing::debug!("count_by_date: {:?}", count);
            }
            Err(why) => {
                tracing::debug!("Failed to fetch_count_by_date because {:?}", why);
            }
        }

        tracing::debug!("結束 fetch_count_by_date");
    }

    #[tokio::test]
    async fn test_makeup_for_the_lack_daily_quotes() {
        dotenvy::dotenv().ok();
        SHARE.load().await;

        let now = Local::now().date_naive();

        tracing::debug!("開始 makeup_for_the_lack_daily_quotes");

        match makeup_for_the_lack_daily_quotes(now).await {
            Ok(result) => {
                tracing::debug!("result:{:#?}", result);
            }
            Err(why) => {
                tracing::debug!(
                    "Failed to makeup_for_the_lack_daily_quotes because:{:?}",
                    why
                );
            }
        }

        tracing::debug!("結束 makeup_for_the_lack_daily_quotes");
    }

    #[tokio::test]
    async fn test_fetch_lowest_avg_highest_price() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 fetch_lowest_avg_highest_price");

        match fetch_monthly_stock_price_summary("2330", 2023, 4).await {
            Ok(cd) => {
                tracing::debug!("stock: {:?}", cd);
            }
            Err(why) => {
                tracing::debug!("Failed to execute because {:?}", why);
            }
        }

        tracing::debug!("結束 fetch_lowest_avg_highest_price");
    }
}
