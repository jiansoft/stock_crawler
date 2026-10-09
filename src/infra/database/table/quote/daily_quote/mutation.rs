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
mod tests;
