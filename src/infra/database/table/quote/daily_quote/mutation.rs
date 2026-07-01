//! `DailyQuote` 的資料庫寫入／更新操作。
//!
//! 包含單筆 upsert、依日期回填均線與年內統計、批次更新均線/PBR，
//! 以及使用 `COPY` 的批次寫入。

use anyhow::{Context, Result, anyhow};
use chrono::TimeDelta;
use sqlx::{Row, postgres::PgQueryResult};

use crate::infra::database;

use super::{COPY_IN_QUERY, DailyQuote};

impl DailyQuote {
    /// 將當前報價寫入資料庫，若主鍵衝突則更新既有資料。
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
            "stock_symbol",
            year,
            month,
            day
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33)
        ON CONFLICT ("stock_symbol", "Date")
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
            "Transaction" = excluded."Transaction",
            "price-to-book_ratio" = excluded."price-to-book_ratio",
            "PriceEarningRatio" = excluded."PriceEarningRatio"
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
            .bind(&self.stock_symbol)
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

    /// 依指定日期回填該股票的均線與年內高低點統計。
    pub async fn fill_moving_average(&mut self) -> Result<()> {
        let year_ago = self.date - TimeDelta::try_days(400).unwrap();
        let sql = r#"
WITH
cte AS (
    SELECT "Date","HighestPrice","LowestPrice","ClosingPrice"
    FROM "DailyQuotes"
    WHERE "stock_symbol" = $1 AND "Date" <= $2 AND "Date" >= $3
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
            .bind(&self.stock_symbol)
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
                "Failed to fetch_moving_average(stock_symbol:{},date:{}) from database",
                self.stock_symbol, self.date
            ))
    }

    /// 批次更新均線、年內統計與 PBR。
    pub async fn batch_update_moving_average(quotes: &[Self]) -> Result<PgQueryResult> {
        if quotes.is_empty() {
            return Err(anyhow!("Cannot batch update empty quotes"));
        }

        let mut serials = Vec::with_capacity(quotes.len());
        let mut ma5 = Vec::with_capacity(quotes.len());
        let mut ma10 = Vec::with_capacity(quotes.len());
        let mut ma20 = Vec::with_capacity(quotes.len());
        let mut ma60 = Vec::with_capacity(quotes.len());
        let mut ma120 = Vec::with_capacity(quotes.len());
        let mut ma240 = Vec::with_capacity(quotes.len());
        let mut max_p = Vec::with_capacity(quotes.len());
        let mut min_p = Vec::with_capacity(quotes.len());
        let mut avg_p = Vec::with_capacity(quotes.len());
        let mut max_d = Vec::with_capacity(quotes.len());
        let mut min_d = Vec::with_capacity(quotes.len());
        let mut pbr = Vec::with_capacity(quotes.len());

        for q in quotes {
            serials.push(q.serial);
            ma5.push(q.moving_average_5);
            ma10.push(q.moving_average_10);
            ma20.push(q.moving_average_20);
            ma60.push(q.moving_average_60);
            ma120.push(q.moving_average_120);
            ma240.push(q.moving_average_240);
            max_p.push(q.maximum_price_in_year);
            min_p.push(q.minimum_price_in_year);
            avg_p.push(q.average_price_in_year);
            max_d.push(q.maximum_price_in_year_date_on);
            min_d.push(q.minimum_price_in_year_date_on);
            pbr.push(q.price_to_book_ratio);
        }

        let sql = r#"
            UPDATE "DailyQuotes" AS dq
            SET
                "MovingAverage5" = t.ma5,
                "MovingAverage10" = t.ma10,
                "MovingAverage20" = t.ma20,
                "MovingAverage60" = t.ma60,
                "MovingAverage120" = t.ma120,
                "MovingAverage240" = t.ma240,
                maximum_price_in_year = t.max_p,
                minimum_price_in_year = t.min_p,
                average_price_in_year = t.avg_p,
                maximum_price_in_year_date_on = t.max_d,
                minimum_price_in_year_date_on = t.min_d,
                "price-to-book_ratio" = t.pbr
            FROM UNNEST($1::bigint[], $2::numeric[], $3::numeric[], $4::numeric[], $5::numeric[], $6::numeric[], $7::numeric[],
                        $8::numeric[], $9::numeric[], $10::numeric[], $11::date[], $12::date[], $13::numeric[])
                 AS t(serial, ma5, ma10, ma20, ma60, ma120, ma240, max_p, min_p, avg_p, max_d, min_d, pbr)
            WHERE dq."Serial" = t.serial
        "#;

        sqlx::query(sql)
            .bind(&serials)
            .bind(&ma5)
            .bind(&ma10)
            .bind(&ma20)
            .bind(&ma60)
            .bind(&ma120)
            .bind(&ma240)
            .bind(&max_p)
            .bind(&min_p)
            .bind(&avg_p)
            .bind(&max_d)
            .bind(&min_d)
            .bind(&pbr)
            .execute(database::get_connection())
            .await
            .context("Failed to batch_update_moving_average in DailyQuotes")
    }

    /// 使用 `COPY` 批次寫入 `DailyQuotes`。
    pub async fn copy_in_raw(quotes: &[Self]) -> Result<u64> {
        database::copy_in_raw(COPY_IN_QUERY, quotes).await
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Local, NaiveDate};

    use crate::core::declare::StockExchange;
    use crate::infra::cache::SHARE;
    use crate::infra::crawler::twse;

    use super::super::FromWithExchange;
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_fetch_moving_average() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 fetch_moving_average");
        let date = NaiveDate::from_ymd_opt(2023, 8, 1);
        let mut dq = DailyQuote::new("2330".to_string());
        dq.date = date.unwrap();
        match dq.fill_moving_average().await {
            Ok(_) => {
                dbg!(&dq);
                tracing::debug!("fetch_moving_average: {:#?}", dq);
            }
            Err(why) => {
                tracing::debug!("Failed to fetch_moving_average because {:?}", why);
            }
        }

        tracing::debug!("結束 fetch_moving_average");
    }

    #[tokio::test]
    async fn test_upsert() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 upsert");

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
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to upsert because:{:?}", why);
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
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to upsert because:{:?}", why);
            }
        }
        tracing::debug!("結束 upsert");
    }

    /// 手動驗證 TWSE 報價抓取後可透過 COPY 寫入測試資料。
    ///
    /// 此測試同時依賴外部網路與本機資料庫，預設測試集不應執行。
    #[tokio::test]
    #[ignore]
    async fn test_copy_in_raw() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_copy_in_raw：無資料庫連接");
            return;
        }
        tracing::debug!("開始 copy_in_raw");

        let date = NaiveDate::from_ymd_opt(2023, 12, 4).unwrap();
        let Ok(twse_dtos) = twse::quote::visit(date).await else {
            println!("跳過 test_copy_in_raw：無法連線 TWSE API");
            return;
        };
        let target_date = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        let mut twse = Vec::new();
        for mut dto in twse_dtos {
            dto.date = target_date;
            let cmd = crate::app::backfill::acl::QuoteAclMapper::from_dto(&dto);
            let entity = crate::app::backfill::acl::QuoteAclMapper::from_command(&cmd);
            twse.push(DailyQuote::from(entity));
        }

        let _ = sqlx::query(r#"delete from "DailyQuotes" where "Date" = $1;"#)
            .bind(target_date)
            .execute(database::get_connection())
            .await;

        match DailyQuote::copy_in_raw(&twse).await {
            Ok(cd) => {
                tracing::debug!("copy_in_raw: {:?}", cd);
            }
            Err(why) => {
                tracing::debug!("Failed to copy_in_raw because {:?}", why);
            }
        }

        tracing::debug!("結束 copy_in_raw");
    }
}
