//! 未來報酬（[`ForwardReturnRepository`]）的 PostgreSQL 實作。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::{
    domain::quote::{
        forward_return::{DerivedCounts, DerivedDataQuery, ForwardReturnRepository},
        relative_strength::CROSSING_INDICATORS,
    },
    infra::database::{self, repository::quote::PgQuoteRepository},
};

/// `$1`（含）之前最近 `$2` 個交易日；走 `"Date" DESC` 索引，回看 400 天足以涵蓋 240 個交易日。
const TRADING_DATES_SQL: &str = r#"
SELECT DISTINCT "Date"
FROM "DailyQuotes"
WHERE "Date" <= $1
  AND "Date" > $1 - 400
ORDER BY "Date" DESC
LIMIT $2
"#;

/// 基準日 `$1` 有成交的上市櫃股票，到結束日 `$2`（含）最近一筆收盤的還原報酬，寫入 `$3` 日的欄位。
///
/// 還原因子＝期間內（基準日之後、結束日之前含）每個除權息或股數變動事件的「參考價 ÷ 前收」
/// 連乘，參考價＝(前收 − 現金股利) ÷ (1 + 股票股利 ÷ 10) ÷ 股數比例，與技術指標的還原相同；
/// 報酬＝結束收盤 ÷ (基準收盤 × 因子) − 1。同日重複的股利（債券 ETF 月配與季配兩列）取最大值。
const UPSERT_SQL: &str = r#"
WITH base AS (
    SELECT q.stock_symbol, q."ClosingPrice" AS base_close
    FROM "DailyQuotes" q
    JOIN stocks s ON s.stock_symbol = q.stock_symbol
    WHERE q."Date" = $1
      AND q."ClosingPrice" > 0
      AND s.stock_exchange_market_id IN (2, 4)
),
ending AS (
    SELECT b.stock_symbol, b.base_close, l.close AS end_close
    FROM base b
    CROSS JOIN LATERAL (
        SELECT q."ClosingPrice" AS close
        FROM "DailyQuotes" q
        WHERE q.stock_symbol = b.stock_symbol
          AND q."Date" > $1 AND q."Date" <= $2
          AND q."ClosingPrice" > 0
        ORDER BY q."Date" DESC
        LIMIT 1
    ) l
),
events AS (
    SELECT security_code AS stock_symbol,
           CASE WHEN "ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$' THEN "ex-dividend_date1"::date END AS date,
           cash_dividend AS cash, 0::numeric AS stock, 1::numeric AS share_ratio
    FROM dividend
    WHERE cash_dividend > 0 AND security_code IN (SELECT stock_symbol FROM ending)
    UNION ALL
    SELECT security_code,
           CASE WHEN "ex-dividend_date2" ~ '^\d{4}-\d{2}-\d{2}$' THEN "ex-dividend_date2"::date END,
           0, stock_dividend, 1
    FROM dividend
    WHERE stock_dividend > 0 AND security_code IN (SELECT stock_symbol FROM ending)
    UNION ALL
    SELECT stock_symbol, effective_date, 0, 0, share_ratio
    FROM corporate_action
    WHERE share_ratio > 0 AND stock_symbol IN (SELECT stock_symbol FROM ending)
),
merged AS (
    SELECT stock_symbol, date, MAX(cash) AS cash, MAX(stock) AS stock,
           COALESCE(MAX(share_ratio) FILTER (WHERE share_ratio <> 1), 1) AS share_ratio
    FROM events
    WHERE date > $1 AND date <= $2
    GROUP BY stock_symbol, date
),
factors AS (
    SELECT m.stock_symbol,
           EXP(SUM(LN((p.prev - m.cash) / (1 + m.stock / 10) / m.share_ratio / p.prev))) AS factor
    FROM merged m
    CROSS JOIN LATERAL (
        SELECT q."ClosingPrice" AS prev
        FROM "DailyQuotes" q
        WHERE q.stock_symbol = m.stock_symbol
          AND q."Date" < m.date
          AND q."ClosingPrice" > 0
        ORDER BY q."Date" DESC
        LIMIT 1
    ) p
    WHERE p.prev > m.cash
    GROUP BY m.stock_symbol
),
returns AS (
    SELECT e.stock_symbol,
           (e.end_close / (e.base_close * COALESCE(f.factor, 1)) - 1) * 100 AS value
    FROM ending e
    LEFT JOIN factors f USING (stock_symbol)
)
INSERT INTO daily_forward_return AS t (date, stock_symbol, return_5d, return_20d, return_60d, return_120d, return_240d)
SELECT $1,
       r.stock_symbol,
       CASE WHEN $3 = 5 THEN r.value END,
       CASE WHEN $3 = 20 THEN r.value END,
       CASE WHEN $3 = 60 THEN r.value END,
       CASE WHEN $3 = 120 THEN r.value END,
       CASE WHEN $3 = 240 THEN r.value END
FROM returns r
ON CONFLICT (date, stock_symbol) DO UPDATE
SET return_5d = COALESCE(EXCLUDED.return_5d, t.return_5d),
    return_20d = COALESCE(EXCLUDED.return_20d, t.return_20d),
    return_60d = COALESCE(EXCLUDED.return_60d, t.return_60d),
    return_120d = COALESCE(EXCLUDED.return_120d, t.return_120d),
    return_240d = COALESCE(EXCLUDED.return_240d, t.return_240d),
    updated_at = CURRENT_TIMESTAMP
"#;

/// 重算基準日 `$1`（陣列）的大盤未來報酬彙總（`daily_market_forward_return`）：上市櫃全部（0）與各市場
/// 的筆數、報酬總和與上漲檔數，口徑與訊號績效原本逐列 `AVG` 的大盤基準相同（只算 `stocks` 有的代號）。
/// 該日已沒有任何未來報酬時，刪掉舊的彙總列。
pub(super) const REFRESH_MARKET_SQL: &str = r#"
WITH fresh AS (
    SELECT r.date, m.market_id,
           COUNT(r.return_5d) AS n5, SUM(r.return_5d) AS sum5,
           SUM(CASE WHEN r.return_5d > 0 THEN 1.0 WHEN r.return_5d IS NOT NULL THEN 0.0 END) AS win5,
           COUNT(r.return_20d) AS n20, SUM(r.return_20d) AS sum20,
           SUM(CASE WHEN r.return_20d > 0 THEN 1.0 WHEN r.return_20d IS NOT NULL THEN 0.0 END) AS win20,
           COUNT(r.return_60d) AS n60, SUM(r.return_60d) AS sum60,
           SUM(CASE WHEN r.return_60d > 0 THEN 1.0 WHEN r.return_60d IS NOT NULL THEN 0.0 END) AS win60,
           COUNT(r.return_120d) AS n120, SUM(r.return_120d) AS sum120,
           SUM(CASE WHEN r.return_120d > 0 THEN 1.0 WHEN r.return_120d IS NOT NULL THEN 0.0 END) AS win120,
           COUNT(r.return_240d) AS n240, SUM(r.return_240d) AS sum240,
           SUM(CASE WHEN r.return_240d > 0 THEN 1.0 WHEN r.return_240d IS NOT NULL THEN 0.0 END) AS win240
    FROM daily_forward_return r
    JOIN stocks s ON s.stock_symbol = r.stock_symbol
    CROSS JOIN LATERAL (VALUES (0::smallint), (s.stock_exchange_market_id::smallint)) AS m(market_id)
    WHERE r.date = ANY($1) AND m.market_id IS NOT NULL
    GROUP BY r.date, m.market_id
),
removed AS (
    DELETE FROM daily_market_forward_return t
    WHERE t.date = ANY($1)
      AND NOT EXISTS (SELECT 1 FROM fresh f WHERE f.date = t.date AND f.market_id = t.market_id)
)
INSERT INTO daily_market_forward_return AS t
    (date, market_id, n5, sum5, win5, n20, sum20, win20, n60, sum60, win60,
     n120, sum120, win120, n240, sum240, win240)
SELECT date, market_id, n5, sum5, win5, n20, sum20, win20, n60, sum60, win60,
       n120, sum120, win120, n240, sum240, win240
FROM fresh
ON CONFLICT (date, market_id) DO UPDATE
SET n5 = EXCLUDED.n5, sum5 = EXCLUDED.sum5, win5 = EXCLUDED.win5,
    n20 = EXCLUDED.n20, sum20 = EXCLUDED.sum20, win20 = EXCLUDED.win20,
    n60 = EXCLUDED.n60, sum60 = EXCLUDED.sum60, win60 = EXCLUDED.win60,
    n120 = EXCLUDED.n120, sum120 = EXCLUDED.sum120, win120 = EXCLUDED.win120,
    n240 = EXCLUDED.n240, sum240 = EXCLUDED.sum240, win240 = EXCLUDED.win240,
    updated_at = CURRENT_TIMESTAMP
"#;

/// 重算一批基準日的大盤未來報酬彙總（見 [`REFRESH_MARKET_SQL`]），回傳寫入的列數。
pub async fn refresh_market_forward_returns(dates: &[NaiveDate]) -> Result<u64> {
    let result = sqlx::query(REFRESH_MARKET_SQL)
        .bind(dates)
        .execute(database::get_connection())
        .await?;
    Ok(result.rows_affected())
}

/// 重算 `from`～`to` 期間內出現在未來報酬或彙總表的每個基準日（彙總表多出來的日期會被刪掉），
/// 一次 [`MARKET_REFRESH_CHUNK`] 天；回傳處理的天數。歷史回補與測試種完資料後使用。
pub async fn refresh_market_forward_returns_between(
    from: NaiveDate,
    to: NaiveDate,
) -> Result<usize> {
    let dates: Vec<NaiveDate> = sqlx::query_scalar(
        r#"SELECT date FROM daily_forward_return WHERE date BETWEEN $1 AND $2
           UNION
           SELECT date FROM daily_market_forward_return WHERE date BETWEEN $1 AND $2
           ORDER BY 1"#,
    )
    .bind(from)
    .bind(to)
    .fetch_all(database::get_connection())
    .await?;
    for chunk in dates.chunks(MARKET_REFRESH_CHUNK) {
        refresh_market_forward_returns(chunk).await?;
    }
    Ok(dates.len())
}

/// 歷史回補每次重算的天數（一天約 2,000 列，100 天約 20 萬列）。
const MARKET_REFRESH_CHUNK: usize = 100;

/// `$1` 的技術訊號筆數（不含相對強度穿越訊號 `$3`，那些隨每日相對強度寫入），以及基準日 `$2`
/// 已寫入 5 日報酬的筆數與 `$1` 的相對強度筆數（都走主鍵前綴）。
const DERIVED_COUNTS_SQL: &str = r#"
SELECT (SELECT COUNT(*) FROM daily_technical_signal WHERE date = $1 AND indicator <> ALL($3)),
       (SELECT COUNT(*) FROM daily_forward_return WHERE date = $2 AND return_5d IS NOT NULL),
       (SELECT COUNT(*) FROM daily_relative_strength WHERE date = $1)
"#;

#[async_trait]
impl DerivedDataQuery for PgQuoteRepository {
    async fn count_derived(&self, date: NaiveDate, base: NaiveDate) -> Result<DerivedCounts> {
        let (signals, forward_returns, relative_strength): (i64, i64, i64) =
            sqlx::query_as(DERIVED_COUNTS_SQL)
                .bind(date)
                .bind(base)
                .bind(&CROSSING_INDICATORS[..])
                .fetch_one(database::get_connection())
                .await?;
        Ok(DerivedCounts {
            signals,
            forward_returns,
            relative_strength,
        })
    }
}

#[async_trait]
impl ForwardReturnRepository for PgQuoteRepository {
    async fn fetch_trading_dates(&self, until: NaiveDate, count: usize) -> Result<Vec<NaiveDate>> {
        let rows: Vec<(NaiveDate,)> = sqlx::query_as(TRADING_DATES_SQL)
            .bind(until)
            .bind(i64::try_from(count).unwrap_or(i64::MAX))
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows.into_iter().map(|(date,)| date).collect())
    }

    async fn upsert_forward_returns(
        &self,
        base: NaiveDate,
        end: NaiveDate,
        horizon: usize,
    ) -> Result<u64> {
        let result = sqlx::query(UPSERT_SQL)
            .bind(base)
            .bind(end)
            .bind(i32::try_from(horizon).unwrap_or(i32::MAX))
            .execute(database::get_connection())
            .await?;
        // 大盤基準讀彙總表，該基準日的報酬一變就跟著重算（約 2,000 列，成本很低）。
        refresh_market_forward_returns(&[base]).await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests;
