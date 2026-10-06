//! `DailyQuote` 的資料庫寫入／更新操作。
//!
//! 包含單筆 upsert、依日期回填均線與年內統計、批次更新均線/PBR，
//! 以及使用 `COPY` 的批次寫入。

use anyhow::{Context, Result, anyhow};
use chrono::{NaiveDate, TimeDelta};
use sqlx::{Row, postgres::PgQueryResult};

use crate::infra::database;

use super::{COPY_IN_QUERY, DailyQuote};

impl DailyQuote {
    /// 將當前報價寫入資料庫，若主鍵衝突則更新既有資料。
    ///
    /// 「upsert」= update + insert：SQL 中的 `ON CONFLICT ("stock_symbol", "Date")
    /// DO UPDATE` 表示——若該股票在該交易日還沒有資料就 INSERT 新增一筆；
    /// 若已存在（撞到唯一索引）則改用 UPDATE 以新值覆蓋價格相關欄位。
    /// 適合單筆補寫；大量寫入請改用 [`Self::copy_in_raw`] 系列以求效能。
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
    ///
    /// 運作方式：SQL 中的 CTE（`WITH cte AS ...`，可想成一張暫時的查詢結果表）
    /// 先取出該股票截至 `self.date` 往回最多 240 個交易日的行情，再由子查詢分別
    /// 計算 5/10/20/60/120/240 日均線與年內最高、最低、平均價。
    /// 樣本數不足時（例如新上市股票不滿 240 筆）該均線以 0 表示，
    /// 計算結果直接回填到 `self` 的對應欄位。
    pub async fn fill_moving_average(&mut self) -> Result<()> {
        // 往回抓 400 個「日曆日」作為查詢下限——扣除週末與假日後，
        // 足以涵蓋 240 個「交易日」的樣本需求。
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

    /// 以視窗函數重算指定股票自 `from` 起（含）所有日報價的均線與年內統計。
    ///
    /// 語意與 [`Self::fill_moving_average`] 相同：每一列取「往回 400 個日曆日內、
    /// 最近 240 筆」為樣本，N 日均線在樣本不足 N 筆時記 0，年內最高／最低／平均
    /// 取整個樣本，最高／最低價同價時取最近一天。差別在於這裡一次算完整段歷史，
    /// 給「回補或替換了過去日期的行情」之後使用——補進一天會改變之後最多 240 個
    /// 交易日的均線，逐日呼叫 [`Self::fill_moving_average`] 太慢。
    ///
    /// 只更新數值有變的列（`IS DISTINCT FROM`），重跑不會產生多餘寫入；
    /// 股價淨值比與日報價本身不動。回傳實際更新的列數。
    pub async fn recalculate_moving_averages(
        stock_symbols: &[String],
        from: NaiveDate,
    ) -> Result<u64> {
        if stock_symbols.is_empty() {
            return Ok(0);
        }

        let sql = r#"
WITH src AS (
    SELECT "Serial", stock_symbol, "Date", "HighestPrice" AS h, "LowestPrice" AS l, "ClosingPrice" AS c
    FROM "DailyQuotes"
    WHERE stock_symbol = ANY($1) AND "Date" >= $2::date - 400
), w AS (
    SELECT "Serial", "Date",
        count(*) OVER r400 AS n,
        avg(c) OVER (sd ROWS 4 PRECEDING) AS a5,
        avg(c) OVER (sd ROWS 9 PRECEDING) AS a10,
        avg(c) OVER (sd ROWS 19 PRECEDING) AS a20,
        avg(c) OVER (sd ROWS 59 PRECEDING) AS a60,
        avg(c) OVER (sd ROWS 119 PRECEDING) AS a120,
        avg(c) OVER (sd ROWS 239 PRECEDING) AS a240,
        max(h) OVER (sd ROWS 239 PRECEDING) AS max_r,
        min(l) OVER (sd ROWS 239 PRECEDING) AS min_r,
        max(h) OVER r400 AS max_d,
        min(l) OVER r400 AS min_d,
        avg(c) OVER r400 AS avg_d,
        max(round(h * 10000) * 100000 + ("Date" - DATE '1900-01-01')) OVER (sd ROWS 239 PRECEDING) AS kmax_r,
        max(round(h * 10000) * 100000 + ("Date" - DATE '1900-01-01')) OVER r400 AS kmax_d,
        max(round((10000000 - l) * 10000) * 100000 + ("Date" - DATE '1900-01-01')) OVER (sd ROWS 239 PRECEDING) AS kmin_r,
        max(round((10000000 - l) * 10000) * 100000 + ("Date" - DATE '1900-01-01')) OVER r400 AS kmin_d
    FROM src
    WINDOW sd AS (PARTITION BY stock_symbol ORDER BY "Date"),
           r400 AS (sd RANGE BETWEEN INTERVAL '400 days' PRECEDING AND CURRENT ROW)
), calc AS (
    SELECT "Serial",
        CASE WHEN n >= 5 THEN round(a5, 2) ELSE 0 END AS ma5,
        CASE WHEN n >= 10 THEN round(a10, 2) ELSE 0 END AS ma10,
        CASE WHEN n >= 20 THEN round(a20, 2) ELSE 0 END AS ma20,
        CASE WHEN n >= 60 THEN round(a60, 2) ELSE 0 END AS ma60,
        CASE WHEN n >= 120 THEN round(a120, 2) ELSE 0 END AS ma120,
        CASE WHEN n >= 240 THEN round(a240, 2) ELSE 0 END AS ma240,
        round(CASE WHEN n >= 240 THEN max_r ELSE max_d END, 2) AS max_p,
        round(CASE WHEN n >= 240 THEN min_r ELSE min_d END, 2) AS min_p,
        round(CASE WHEN n >= 240 THEN a240 ELSE avg_d END, 2) AS avg_p,
        DATE '1900-01-01' + mod(CASE WHEN n >= 240 THEN kmax_r ELSE kmax_d END, 100000)::int AS max_on,
        DATE '1900-01-01' + mod(CASE WHEN n >= 240 THEN kmin_r ELSE kmin_d END, 100000)::int AS min_on
    FROM w
    WHERE "Date" >= $2
)
UPDATE "DailyQuotes" AS dq
SET
    "MovingAverage5" = k.ma5,
    "MovingAverage10" = k.ma10,
    "MovingAverage20" = k.ma20,
    "MovingAverage60" = k.ma60,
    "MovingAverage120" = k.ma120,
    "MovingAverage240" = k.ma240,
    maximum_price_in_year = k.max_p,
    minimum_price_in_year = k.min_p,
    average_price_in_year = k.avg_p,
    maximum_price_in_year_date_on = k.max_on,
    minimum_price_in_year_date_on = k.min_on
FROM calc AS k
WHERE dq."Serial" = k."Serial"
  AND (dq."MovingAverage5", dq."MovingAverage10", dq."MovingAverage20", dq."MovingAverage60",
       dq."MovingAverage120", dq."MovingAverage240", dq.maximum_price_in_year,
       dq.minimum_price_in_year, dq.average_price_in_year,
       dq.maximum_price_in_year_date_on, dq.minimum_price_in_year_date_on)
      IS DISTINCT FROM
      (k.ma5, k.ma10, k.ma20, k.ma60, k.ma120, k.ma240, k.max_p, k.min_p, k.avg_p,
       k.max_on, k.min_on)
"#;

        let result = sqlx::query(sql)
            .bind(stock_symbols)
            .bind(from)
            .execute(database::get_connection())
            .await
            .with_context(|| {
                format!(
                    "Failed to recalculate_moving_averages({} symbols, from {from})",
                    stock_symbols.len()
                )
            })?;
        Ok(result.rows_affected())
    }

    /// 批次更新均線、年內統計與 PBR。
    ///
    /// 效能技巧：把每個欄位各自蒐集成一個 Vec，以陣列參數一次綁定給 SQL 的
    /// `UNNEST`（把多個平行陣列展開成一張暫時資料表），再用單一 `UPDATE ... FROM`
    /// 完成全部更新。這樣不論幾千筆都只需要一次資料庫往返，
    /// 遠快於逐筆執行 UPDATE。
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
    ///
    /// 這個版本會自行從連線池取連線，屬於獨立的自動提交操作；
    /// 若需要與其他 SQL（例如先刪除同日資料）綁在同一個 transaction 內，
    /// 請改用 [`Self::copy_in_raw_on`]。
    pub async fn copy_in_raw(quotes: &[Self]) -> Result<u64> {
        database::copy_in_raw(COPY_IN_QUERY, quotes).await
    }

    /// 在「呼叫端指定的連線」上使用 `COPY` 批次寫入 `DailyQuotes`。
    ///
    /// 呼叫端把 transaction 的連線（`&mut *tx`）傳進來時，這批寫入就會
    /// 跟同一個 transaction 內的其他 SQL 一起 commit 或 rollback。
    /// 「日報價原子替換」（先刪後寫、同生共死）正是靠這個函式達成：
    /// 中途任何一步失敗，資料庫都會回到 transaction 開始前的狀態。
    pub async fn copy_in_raw_on(conn: &mut sqlx::PgConnection, quotes: &[Self]) -> Result<u64> {
        database::copy_in_raw_on(conn, COPY_IN_QUERY, quotes).await
    }

    /// 批次補寫缺漏的日報價，已存在者原封不動，回傳實際新增的列數。
    ///
    /// 專供回補歷史缺口使用，因此是 `ON CONFLICT DO NOTHING` 而非 upsert：
    /// 補洞不該覆寫既有資料 —— 來源（`STOCK_DAY`）沒有本益比、股價淨值比與
    /// 最佳買賣揭示，若用 `DO UPDATE` 會把既有列的那些欄位清成 0。
    /// `COPY` 在這裡也不能用：它不支援 `ON CONFLICT`，區間內只要有一天已存在
    /// 就會整批因唯一索引而失敗。
    pub async fn insert_missing_batch(quotes: &[Self]) -> Result<u64> {
        if quotes.is_empty() {
            return Ok(0);
        }

        let mut builder = sqlx::QueryBuilder::new(
            r#"INSERT INTO "DailyQuotes" (
                "Date", stock_symbol, year, month, day,
                "OpeningPrice", "HighestPrice", "LowestPrice", "ClosingPrice",
                "Change", "ChangeRange", "TradingVolume", "TradeValue", "Transaction",
                maximum_price_in_year_date_on, minimum_price_in_year_date_on
            ) "#,
        );

        builder.push_values(quotes, |mut row, quote| {
            row.push_bind(quote.date)
                .push_bind(quote.stock_symbol.clone())
                .push_bind(quote.year)
                .push_bind(quote.month)
                .push_bind(quote.day)
                .push_bind(quote.opening_price)
                .push_bind(quote.highest_price)
                .push_bind(quote.lowest_price)
                .push_bind(quote.closing_price)
                .push_bind(quote.change)
                .push_bind(quote.change_range)
                .push_bind(quote.trading_volume)
                .push_bind(quote.trade_value)
                .push_bind(quote.transaction)
                .push_bind(quote.maximum_price_in_year_date_on)
                .push_bind(quote.minimum_price_in_year_date_on);
        });
        builder.push(r#" ON CONFLICT (stock_symbol, "Date") DO NOTHING"#);

        let result = builder
            .build()
            .execute(database::get_connection())
            .await
            .context("Failed to insert missing daily quotes")?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Local, NaiveDate};
    use rust_decimal::Decimal;

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
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
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

        // 測試結束後移除假代號資料列，避免測試資料殘留在資料庫
        // （曾在正式庫發現本測試留下的 '79979' 收盤紀錄）。
        if let Err(why) = sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE "stock_symbol" = $1"#)
            .bind("79979")
            .execute(database::get_connection())
            .await
        {
            tracing::debug!("Failed to cleanup test rows because:{:?}", why);
        }

        tracing::debug!("結束 upsert");
    }

    /// 空批次沒有任何可更新的目標，必須明確報錯而不是送出一句無效 SQL。
    #[tokio::test]
    async fn batch_update_moving_average_rejects_an_empty_batch() {
        let err = DailyQuote::batch_update_moving_average(&[])
            .await
            .expect_err("空批次應回傳錯誤");
        assert!(
            err.to_string().contains("empty"),
            "錯誤訊息應說明原因：{err}"
        );
    }

    /// 空批次在碰資料庫之前就短路回 0，回補流程可以放心傳空清單。
    #[tokio::test]
    async fn insert_missing_batch_short_circuits_on_an_empty_slice() {
        assert_eq!(
            DailyQuote::insert_missing_batch(&[])
                .await
                .expect("空批次應成功"),
            0
        );
    }

    /// 建立一筆最小可寫入的測試報價（固定歷史日期 + 假代號）。
    fn make_quote(
        symbol: &str,
        date: NaiveDate,
        closing_price: rust_decimal::Decimal,
    ) -> DailyQuote {
        let mut quote = DailyQuote::new(symbol.to_string());
        quote.date = date;
        quote.year = date.year();
        quote.month = date.month() as i32;
        quote.day = date.day() as i32;
        quote.closing_price = closing_price;
        quote.opening_price = closing_price;
        quote.highest_price = closing_price;
        quote.lowest_price = closing_price;
        quote
    }

    /// 回補歷史缺口時只補空位：新日期寫入，既有日期既不重複也不被覆寫。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn insert_missing_batch_fills_gaps_without_overwriting() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 insert_missing_batch_fills_gaps_without_overwriting：無資料庫連接");
            return;
        }

        // 固定歷史日期 + 假代號，避免碰到任何真實資料。
        const SYMBOL: &str = "79979";
        let day1 = NaiveDate::from_ymd_opt(1970, 1, 1).expect("測試日期應合法");
        let day2 = NaiveDate::from_ymd_opt(1970, 1, 2).expect("測試日期應合法");
        let cleanup = || async {
            let _ = sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE "stock_symbol" = $1"#)
                .bind(SYMBOL)
                .execute(database::get_connection())
                .await;
        };
        cleanup().await;

        // 首次寫入兩天，兩筆都是新資料。
        let inserted = DailyQuote::insert_missing_batch(&[
            make_quote(SYMBOL, day1, rust_decimal::Decimal::from(10)),
            make_quote(SYMBOL, day2, rust_decimal::Decimal::from(11)),
        ])
        .await
        .expect("首次寫入應成功");
        assert_eq!(inserted, 2);

        // 重跑同一批並多帶一天：只有新的那天會被寫入（ON CONFLICT DO NOTHING）。
        let day3 = NaiveDate::from_ymd_opt(1970, 1, 3).expect("測試日期應合法");
        let inserted = DailyQuote::insert_missing_batch(&[
            make_quote(SYMBOL, day1, rust_decimal::Decimal::from(99)),
            make_quote(SYMBOL, day2, rust_decimal::Decimal::from(99)),
            make_quote(SYMBOL, day3, rust_decimal::Decimal::from(12)),
        ])
        .await
        .expect("重跑應成功");
        assert_eq!(inserted, 1);

        // 既有列的價格不得被重跑的值覆寫——補洞不該動到已有資料。
        let closing: rust_decimal::Decimal = sqlx::query_scalar(
            r#"SELECT "ClosingPrice" FROM "DailyQuotes" WHERE "stock_symbol" = $1 AND "Date" = $2"#,
        )
        .bind(SYMBOL)
        .bind(day1)
        .fetch_one(database::get_connection())
        .await
        .expect("應查得到既有資料");
        assert_eq!(closing, rust_decimal::Decimal::from(10));

        cleanup().await;
    }

    /// COPY 寫入驗證：以 TWSE 每日收盤行情 fixture（100 檔）走 ACL 轉成 [`DailyQuote`]，
    /// 在交易內 COPY、驗證筆數後回滾，不留下任何資料列。
    ///
    /// 舊版改抓 TWSE 即時資料、COPY 後不清理，且標成 `#[ignore]` 平常不跑；2026-06-30 對正式庫
    /// 執行後留下 1,204 筆日期為 1970-01-01 的重複報價（2026-10-05 才清除）。現在不連外部網站，
    /// 隨整合測試一起執行。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_copy_in_raw_rolls_back() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_copy_in_raw_rolls_back：無資料庫連接");
            return;
        }

        let response: twse::quote::ListedResponse = serde_json::from_str(include_str!(
            "../../../../../../tests/fixtures/twse_quote.json"
        ))
        .expect("TWSE fixture should parse");
        let table = &response.tables[0];
        let field_map: std::collections::HashMap<&str, usize> = table
            .fields
            .as_ref()
            .expect("fixture fields")
            .iter()
            .enumerate()
            .map(|(index, field)| (field.as_str(), index))
            .collect();
        // 用不會與真實資料衝突的日期，避免撞到 (stock_symbol, Date) 唯一索引；交易結束即回滾。
        let date = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        let quotes: Vec<DailyQuote> = table
            .data
            .as_ref()
            .expect("fixture rows")
            .iter()
            .map(|row| {
                let dto = crate::infra::crawler::share::DailyQuoteDto::from_with_map(
                    row, &field_map, date,
                )
                .expect("fixture row should parse");
                let cmd = crate::app::backfill::acl::QuoteAclMapper::from_dto(&dto);
                DailyQuote::from(crate::app::backfill::acl::QuoteAclMapper::from_command(
                    &cmd,
                ))
            })
            .collect();
        assert!(!quotes.is_empty());

        let count_on_date = r#"SELECT COUNT(*) FROM "DailyQuotes" WHERE "Date" = $1"#;
        let mut tx = database::get_connection()
            .begin()
            .await
            .expect("begin transaction");
        let copied = DailyQuote::copy_in_raw_on(&mut tx, &quotes)
            .await
            .expect("copy_in_raw_on should succeed");
        assert_eq!(copied as usize, quotes.len());
        let inside: i64 = sqlx::query_scalar(count_on_date)
            .bind(date)
            .fetch_one(&mut *tx)
            .await
            .expect("count inside transaction");
        assert_eq!(
            inside as usize,
            quotes.len(),
            "交易內應看得到剛 COPY 的資料"
        );
        tx.rollback().await.expect("rollback");

        let after: i64 = sqlx::query_scalar(count_on_date)
            .bind(date)
            .fetch_one(database::get_connection())
            .await
            .expect("count after rollback");
        assert_eq!(after, 0, "回滾後不可留下任何資料列");
    }

    /// 視窗函數重算的結果必須與逐日的 `fill_moving_average` 完全一致。
    ///
    /// 79979 是連續 300 個平日（走 240 筆分支）；79978 每 3 個平日才一筆、跨兩年多
    /// （400 天內不滿 240 筆，走日曆日分支）。收盤價刻意帶小數與重複值，涵蓋同價取日期。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn recalculate_moving_averages_matches_fill_moving_average() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 recalculate_moving_averages_matches_fill_moving_average：無資料庫連接");
            return;
        }

        let symbols = vec!["79979".to_string(), "79978".to_string()];
        let cleanup = || async {
            sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#)
                .bind(&symbols)
                .execute(database::get_connection())
                .await
                .expect("清除測試資料列");
        };
        cleanup().await;

        let insert = r#"
INSERT INTO "DailyQuotes" (stock_symbol, "Date", "ClosingPrice", "HighestPrice", "LowestPrice", year, month, day)
SELECT $1, d, c, c + (i % 4) * 0.5, c - (i % 3) * 0.25, date_part('year', d), date_part('month', d), date_part('day', d)
FROM (
    SELECT row_number() OVER (ORDER BY d) AS i, d::date AS d
    FROM generate_series(DATE '2001-01-01', DATE '2004-12-31', INTERVAL '1 day') AS d
    WHERE extract(isodow FROM d) < 6
) AS days,
LATERAL (SELECT 10 + (i % 17) + (i % 3) * 0.25 AS c) AS price
WHERE i % $2 = 0 AND i <= $3
"#;
        for (symbol, step, last) in [("79979", 1_i64, 300_i64), ("79978", 3, 900)] {
            sqlx::query(insert)
                .bind(symbol)
                .bind(step)
                .bind(last)
                .execute(database::get_connection())
                .await
                .expect("寫入測試資料列");
        }

        let from = NaiveDate::from_ymd_opt(2001, 1, 1).expect("日期應合法");
        let updated = DailyQuote::recalculate_moving_averages(&symbols, from)
            .await
            .expect("重算應成功");
        assert_eq!(updated, 600, "兩檔各 300 列都從全 0 變成有值");

        let again = DailyQuote::recalculate_moving_averages(&symbols, from)
            .await
            .expect("重算應成功");
        assert_eq!(again, 0, "重跑時數值沒變，不該再寫入");

        let dates: Vec<NaiveDate> = sqlx::query_scalar(
            r#"SELECT DISTINCT "Date" FROM "DailyQuotes" WHERE stock_symbol = ANY($1) ORDER BY 1"#,
        )
        .bind(&symbols)
        .fetch_all(database::get_connection())
        .await
        .expect("讀回測試日期");
        let mut rows = Vec::new();
        for date in dates {
            rows.extend(
                super::super::fetch_daily_quotes_by_date(date)
                    .await
                    .expect("讀回測試資料列")
                    .into_iter()
                    .filter(|row| symbols.contains(&row.stock_symbol)),
            );
        }
        assert_eq!(rows.len(), 600);

        let mut mismatches = Vec::new();
        for row in &rows {
            let mut expected = row.clone();
            expected
                .fill_moving_average()
                .await
                .expect("逐日計算應成功");
            let actual = [
                row.moving_average_5,
                row.moving_average_10,
                row.moving_average_20,
                row.moving_average_60,
                row.moving_average_120,
                row.moving_average_240,
                row.maximum_price_in_year,
                row.minimum_price_in_year,
                row.average_price_in_year,
            ];
            let wanted = [
                expected.moving_average_5,
                expected.moving_average_10,
                expected.moving_average_20,
                expected.moving_average_60,
                expected.moving_average_120,
                expected.moving_average_240,
                expected.maximum_price_in_year,
                expected.minimum_price_in_year,
                expected.average_price_in_year,
            ];
            // 同價時逐日算法取哪一天沒有定義，比對「該日的價格等於極值」即可。
            let price_on = |date: NaiveDate, pick: fn(&DailyQuote) -> Decimal| {
                rows.iter()
                    .find(|r| r.stock_symbol == row.stock_symbol && r.date == date)
                    .map(|r| pick(r).round_dp(2))
            };
            let max_on_ok = price_on(row.maximum_price_in_year_date_on, |r| r.highest_price)
                == Some(row.maximum_price_in_year);
            let min_on_ok = price_on(row.minimum_price_in_year_date_on, |r| r.lowest_price)
                == Some(row.minimum_price_in_year);
            if actual != wanted || !max_on_ok || !min_on_ok {
                mismatches.push((row.stock_symbol.clone(), row.date));
            }
        }

        cleanup().await;
        assert!(mismatches.is_empty(), "與逐日算法不一致：{mismatches:?}");
    }
}
