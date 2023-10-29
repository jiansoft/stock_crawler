use core::result::Result::Ok;

use anyhow::{anyhow, Context, Result};
use chrono::{Duration, Local, NaiveDate};
use rust_decimal::Decimal;
use sqlx::postgres::PgQueryResult;

use crate::internal::database;

#[derive(sqlx::FromRow, Debug)]
/// 最後交易日股票報價數據
pub struct LastDailyQuotes {
    pub date: NaiveDate,
    pub security_code: String,
    /// 收盤價
    pub closing_price: Decimal,
}

impl LastDailyQuotes {
    pub fn new() -> Self {
        LastDailyQuotes {
            date: Default::default(),
            security_code: "".to_string(),
            closing_price: Default::default(),
        }
    }

    /// 取得最後交易日股票報價數據
    pub async fn fetch() -> Result<Vec<LastDailyQuotes>> {
        Ok(sqlx::query_as::<_, LastDailyQuotes>(
            r#"
SELECT
    date, security_code, closing_price
FROM
    last_daily_quotes
"#,
        )
        .fetch_all(database::get_connection())
        .await?)
    }

    pub async fn rebuild() -> Result<PgQueryResult> {
        let mut tx = database::get_tx()
            .await
            .context("Failed to get_tx in last_daily_quotes")?;

        if let Err(why) = sqlx::query("TRUNCATE last_daily_quotes;")
            .execute(&mut *tx)
            .await
            .context("Failed to TRUNCATE last_daily_quotes;")
        {
            tx.rollback().await?;
            return Err(anyhow!("{:?}", why));
        }

        let sql = r#"
INSERT INTO last_daily_quotes
SELECT
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
	"RecordTime",
	current_timestamp
FROM "DailyQuotes"
WHERE "Serial" IN
(
	select max("Serial")
	from "DailyQuotes"
	where "Date" >= $1
	group by "SecurityCode"
)
ORDER BY "SecurityCode"
ON CONFLICT (security_code)
DO UPDATE SET
	trading_volume = excluded.trading_volume,
	transaction = excluded.transaction,
	trade_value = excluded.trade_value,
	opening_price = excluded.opening_price,
	highest_price = excluded.highest_price,
	lowest_price = excluded.lowest_price,
	closing_price = excluded.closing_price,
	change_range = excluded.change_range,
	change = excluded.change,
	last_best_bid_price = excluded.last_best_bid_price,
	last_best_bid_volume = excluded.last_best_bid_volume,
	last_best_ask_price = excluded.last_best_ask_price,
	last_best_ask_volume = excluded.last_best_ask_volume,
	price_earning_ratio = excluded.price_earning_ratio,
	moving_average_5 = excluded.moving_average_5,
	moving_average_10 = excluded.moving_average_10,
	moving_average_20 = excluded.moving_average_20,
	moving_average_60 = excluded.moving_average_60,
	moving_average_120 = excluded.moving_average_120,
	moving_average_240 = excluded.moving_average_240,
	maximum_price_in_year = excluded.maximum_price_in_year,
	minimum_price_in_year = excluded.minimum_price_in_year,
	average_price_in_year = excluded.average_price_in_year,
	maximum_price_in_year_date_on = excluded.maximum_price_in_year_date_on,
	minimum_price_in_year_date_on = excluded.minimum_price_in_year_date_on,
	"price-to-book_ratio" = excluded."price-to-book_ratio",
	record_time = excluded.record_time,
	updated_time  = excluded.updated_time;
"#;
        let year_ago = Local::now() - Duration::days(365);
        match sqlx::query(sql)
            .bind(year_ago)
            .execute(&mut *tx)
            .await
            .context("Failed to LastDailyQuotes::rebuild from database")
        {
            Ok(pg) => {
                tx.commit().await?;
                Ok(pg)
            }
            Err(why) => {
                tx.rollback().await?;
                Err(anyhow!("{:?}", why))
            }
        }
    }

    pub fn clone(&self) -> Self {
        LastDailyQuotes {
            date: self.date,
            security_code: self.security_code.clone(),
            closing_price: self.closing_price,
        }
    }
}

impl Default for LastDailyQuotes {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    async fn test_calculate() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 fetch".to_string());
        let _ = LastDailyQuotes::new();
        match LastDailyQuotes::fetch().await {
            Ok(stocks) => logging::info_file_async(format!("{:#?}", stocks)),
            Err(why) => {
                logging::error_file_async(format!("Failed to fetch because {:?}", why));
            }
        }

        logging::info_file_async("結束 fetch".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_rebuild() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 rebuild".to_string());
        let _ = LastDailyQuotes::new();
        match LastDailyQuotes::rebuild().await {
            Ok(r) => logging::info_file_async(format!("{:#?}", r)),
            Err(why) => {
                logging::error_file_async(format!("Failed to rebuild because {:?}", why));
            }
        }

        logging::info_file_async("結束 rebuild".to_string());
    }
}
