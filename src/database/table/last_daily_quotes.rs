use anyhow::{anyhow, Context, Result};
use chrono::{Local, NaiveDate, TimeDelta};
use rust_decimal::Decimal;
use sqlx::postgres::PgQueryResult;

use crate::database;

#[derive(sqlx::FromRow, Debug)]
/// 最後交易日股票報價數據
pub struct LastDailyQuotes {
    pub date: NaiveDate,
    /// 收盤價
    pub closing_price: Decimal,
    pub stock_symbol: String,
}

impl LastDailyQuotes {
    pub fn new() -> Self {
        LastDailyQuotes {
            date: Default::default(),
            closing_price: Default::default(),
            stock_symbol: Default::default(),
        }
    }

    /// 取得最後交易日股票報價數據
    pub async fn fetch() -> Result<Vec<LastDailyQuotes>> {
        Ok(sqlx::query_as::<_, LastDailyQuotes>(
            r#"
SELECT
    date, stock_symbol, closing_price
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
	group by "stock_symbol"
)
ORDER BY "stock_symbol"
"#;
        let month_ago = Local::now() - TimeDelta::try_days(30).unwrap();
        match sqlx::query(sql)
            .bind(month_ago)
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
}

impl Clone for LastDailyQuotes {
    fn clone(&self) -> Self {
        LastDailyQuotes {
            date: self.date,
            stock_symbol: self.stock_symbol.clone(),
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
    #[ignore]
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
        match LastDailyQuotes::rebuild().await {
            Ok(r) => logging::info_file_async(format!("{:#?}", r)),
            Err(why) => {
                logging::error_file_async(format!("Failed to rebuild because {:?}", why));
            }
        }

        logging::info_file_async("結束 rebuild".to_string());
    }
}
