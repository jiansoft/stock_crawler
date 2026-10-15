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
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::*;

    /// (代號, 5 日, 20 日, 60 日, 240 日報酬)。
    type ReturnRow = (
        String,
        Option<Decimal>,
        Option<Decimal>,
        Option<Decimal>,
        Option<Decimal>,
    );

    /// 交易日由新到舊；報酬依除息與減資還原，不同天期寫在各自欄位、互不覆蓋。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn forward_returns_adjust_for_events() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 forward_returns_adjust_for_events：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        let day = |d| NaiveDate::from_ymd_opt(1991, 8, d).expect("有效日期");
        let symbols = ["79979FA", "79979FB", "79979FC", "79979FD"];
        let cleanup = || async {
            for sql in [
                r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
                "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
                "DELETE FROM dividend WHERE security_code = ANY($1)",
                "DELETE FROM corporate_action WHERE stock_symbol = ANY($1)",
                "DELETE FROM daily_forward_return WHERE stock_symbol = ANY($1)",
                "DELETE FROM daily_technical_signal WHERE stock_symbol = ANY($1)",
            ] {
                sqlx::query(sql)
                    .bind(symbols)
                    .execute(pool)
                    .await
                    .expect("清理測試資料");
            }
        };
        cleanup().await;

        // FA：8/2 收 100，8/6 除息 10 元（前收 100 → 參考價 90），8/8 收 99 → 還原報酬 +10%。
        // FB：8/6 減資五成（1 股變 0.5 股），8/8 收 220 → 前收 100 × 2 = 200，報酬 +10%。
        // FC：基準日沒有成交，不寫入；FD 是興櫃，不寫入。
        for (symbol, market, quotes) in [
            (
                "79979FA",
                2,
                vec![(2, dec!(100)), (5, dec!(100)), (6, dec!(95)), (8, dec!(99))],
            ),
            (
                "79979FB",
                4,
                vec![(2, dec!(100)), (5, dec!(100)), (8, dec!(220))],
            ),
            ("79979FC", 2, vec![(5, dec!(10)), (8, dec!(11))]),
            ("79979FD", 5, vec![(2, dec!(10)), (8, dec!(11))]),
        ] {
            sqlx::query(
                r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
                   VALUES ($1, $1, $1, $2, 1)"#,
            )
            .bind(symbol)
            .bind(market)
            .execute(pool)
            .await
            .expect("寫入測試股票");
            for (d, close) in quotes {
                sqlx::query(
                    r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice") VALUES ($1, $2, $3)"#,
                )
                .bind(day(d))
                .bind(symbol)
                .bind(close)
                .execute(pool)
                .await
                .expect("寫入測試日線");
            }
        }
        sqlx::query(
            r#"INSERT INTO dividend (security_code, year, quarter, cash_dividend, "ex-dividend_date1", "ex-dividend_date2")
               VALUES ('79979FA', 1991, '', 10, '1991-08-06', '-'),
                      ('79979FA', 1990, '', 3, '1991-08-01', '-')"#,
        )
        .execute(pool)
        .await
        .expect("寫入測試股利");
        sqlx::query(
            "INSERT INTO corporate_action (stock_symbol, effective_date, action_type, share_ratio)
             VALUES ('79979FB', '1991-08-06', 'capital_reduction', 0.5)",
        )
        .execute(pool)
        .await
        .expect("寫入測試公司行動");

        let repo = PgQuoteRepository::new();
        let dates = repo
            .fetch_trading_dates(day(8), 3)
            .await
            .expect("查詢交易日");
        assert_eq!(dates, [day(8), day(6), day(5)]);
        // 只有相對強度穿越訊號時，技術訊號仍算缺漏。
        sqlx::query(
            "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
             VALUES ($1, '79979FA', 'rs50', 'cross_above', '相對強度 40 → 60 分，突破 50 分')",
        )
        .bind(day(8))
        .execute(pool)
        .await
        .expect("寫入相對強度訊號");
        let before = repo
            .count_derived(day(8), day(2))
            .await
            .expect("寫入前的筆數");
        assert_eq!((before.signals, before.forward_returns), (0, 0));

        assert_eq!(
            repo.upsert_forward_returns(day(2), day(8), 5)
                .await
                .expect("寫入 5 日"),
            2
        );
        repo.upsert_forward_returns(day(2), day(5), 20)
            .await
            .expect("寫入 20 日");
        repo.upsert_forward_returns(day(2), day(8), 240)
            .await
            .expect("寫入 240 日");
        // 重算 5 日不影響 20 日欄位。
        repo.upsert_forward_returns(day(2), day(8), 5)
            .await
            .expect("重算 5 日");

        let counts = repo
            .count_derived(day(8), day(2))
            .await
            .expect("寫入後的筆數");
        assert_eq!(counts.forward_returns, 2);
        let rows: Vec<ReturnRow> = sqlx::query_as(
            "SELECT stock_symbol, return_5d, return_20d, return_60d, return_240d FROM daily_forward_return
                 WHERE stock_symbol = ANY($1) ORDER BY 1",
        )
        .bind(symbols)
        .fetch_all(pool)
        .await
        .expect("讀回報酬");
        cleanup().await;
        assert_eq!(
            rows,
            [
                (
                    "79979FA".to_owned(),
                    Some(dec!(10)),
                    Some(dec!(0)),
                    None,
                    Some(dec!(10))
                ),
                (
                    "79979FB".to_owned(),
                    Some(dec!(10)),
                    Some(dec!(0)),
                    None,
                    Some(dec!(10))
                ),
            ]
        );
    }
}
