//! 資料健康檢查的 PostgreSQL 量測實作。

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::{
    domain::{
        health::{DataHealthRepository, DataHealthSnapshot, DerivedTableLatest},
        quote::entity::MarketTradedCount,
    },
    infra::database::{self, table::daily_quote},
};

/// 每日收盤後會更新的衍生資料表，與各自的日期欄位。
///
/// 表名與欄位名無法用參數綁定，只能寫死在這裡；新增衍生表時同步加入。
const DERIVED_TABLES: [(&str, &str); 6] = [
    ("estimate", "SELECT max(date) FROM estimate"),
    ("yield_rank", "SELECT max(date) FROM yield_rank"),
    (
        "daily_stock_price_stats",
        "SELECT max(date) FROM daily_stock_price_stats",
    ),
    (
        "last_daily_quotes",
        "SELECT max(date) FROM last_daily_quotes",
    ),
    ("stock_cagr", "SELECT max(date) FROM stock_cagr"),
    ("qfii_history", "SELECT max(date) FROM qfii_history"),
];

/// 檢查期間內有收盤價、前 400 天內已有至少 5 筆報價，5 日均線卻仍是 0 的列數。
const MISSING_MOVING_AVERAGES_SQL: &str = r#"
SELECT count(*)
FROM "DailyQuotes" AS d
WHERE d."Date" BETWEEN $1 AND $2
  AND d."ClosingPrice" > 0
  AND d."MovingAverage5" = 0
  AND (
      SELECT count(*)
      FROM (
          SELECT 1
          FROM "DailyQuotes" AS p
          WHERE p.stock_symbol = d.stock_symbol
            AND p."Date" <= d."Date"
            AND p."Date" >= d."Date" - 400
          LIMIT 5
      ) AS recent
  ) = 5
"#;

/// 漲跌幅與「漲跌 ÷ 參考價（收盤 − 漲跌）」不符的列數；漲跌為 0 時漲跌幅也必須是 0。
const CHANGE_RANGE_MISMATCHES_SQL: &str = r#"
SELECT count(*)
FROM "DailyQuotes"
WHERE "Date" BETWEEN $1 AND $2
  AND CASE
          WHEN "Change" = 0 OR "ClosingPrice" - "Change" <= 0 THEN "ChangeRange" <> 0
          ELSE abs("ChangeRange" - "Change" / ("ClosingPrice" - "Change") * 100) > 0.01
      END
"#;

/// `year`／`month`／`day` 與日期不符的列數（補值列曾漏填成 0）。
const MISDATED_ROWS_SQL: &str = r#"
SELECT count(*)
FROM "DailyQuotes"
WHERE "Date" BETWEEN $1 AND $2
  AND (year, month, day) IS DISTINCT FROM (
      date_part('year', "Date")::int,
      date_part('month', "Date")::int,
      date_part('day', "Date")::int
  )
"#;

/// 發放年度早於今年、除權息日仍是「尚未公布」的股利列數。
const STALE_DIVIDEND_PLACEHOLDERS_SQL: &str = r#"
SELECT count(*)
FROM dividend
WHERE year < $1
  AND ("ex-dividend_date1" = '尚未公布' OR "ex-dividend_date2" = '尚未公布')
"#;

/// 年度合計列與各期明細加總不符的組數（只比對有明細的年度）。
const DIVIDEND_TOTAL_MISMATCHES_SQL: &str = r#"
WITH details AS (
    SELECT security_code, year,
           sum(cash_dividend) AS cash_dividend,
           sum(stock_dividend) AS stock_dividend
    FROM dividend
    WHERE quarter <> ''
    GROUP BY security_code, year
)
SELECT count(*)
FROM dividend AS total
JOIN details USING (security_code, year)
WHERE total.quarter = ''
  AND (abs(total.cash_dividend - details.cash_dividend) > 0.0001
       OR abs(total.stock_dividend - details.stock_dividend) > 0.0001)
"#;

/// 基於 PostgreSQL 的資料健康檢查量測。
#[derive(Debug, Default, Clone, Copy)]
pub struct PgDataHealthRepository;

impl PgDataHealthRepository {
    /// 建立新的量測實例。
    pub fn new() -> Self {
        Self
    }
}

/// 執行只回傳一個 `count(*)` 的查詢。
async fn count_between(
    sql: &'static str,
    from: NaiveDate,
    to: NaiveDate,
    label: &str,
) -> Result<i64> {
    sqlx::query_scalar(sql)
        .bind(from)
        .bind(to)
        .fetch_one(database::get_connection())
        .await
        .with_context(|| format!("Failed to count {label} {from}~{to}"))
}

#[async_trait]
impl DataHealthRepository for PgDataHealthRepository {
    async fn fetch_snapshot(
        &self,
        baseline_from: NaiveDate,
        from: NaiveDate,
        to: NaiveDate,
        current_year: i32,
    ) -> Result<DataHealthSnapshot> {
        let pool = database::get_connection();

        let market_counts = daily_quote::fetch_market_traded_counts(baseline_from, to)
            .await?
            .into_iter()
            .map(|(date, market_id, traded)| MarketTradedCount {
                date,
                market_id,
                traded,
            })
            .collect();

        let quoted_dates = sqlx::query_scalar(
            r#"SELECT DISTINCT "Date" FROM "DailyQuotes" WHERE "Date" BETWEEN $1 AND $2 ORDER BY 1"#,
        )
        .bind(from)
        .bind(to)
        .fetch_all(pool)
        .await
        .with_context(|| format!("Failed to fetch quoted dates {from}~{to}"))?;

        let mut derived_tables = Vec::with_capacity(DERIVED_TABLES.len());
        for (table, sql) in DERIVED_TABLES {
            let latest: Option<NaiveDate> = sqlx::query_scalar(sql)
                .fetch_one(pool)
                .await
                .with_context(|| format!("Failed to fetch latest date of {table}"))?;
            derived_tables.push(DerivedTableLatest { table, latest });
        }

        let stale_dividend_placeholders = sqlx::query_scalar(STALE_DIVIDEND_PLACEHOLDERS_SQL)
            .bind(current_year)
            .fetch_one(pool)
            .await
            .context("Failed to count stale dividend placeholders")?;

        let dividend_total_mismatches = sqlx::query_scalar(DIVIDEND_TOTAL_MISMATCHES_SQL)
            .fetch_one(pool)
            .await
            .context("Failed to count dividend total mismatches")?;

        let latest_revenue_month = sqlx::query_scalar(r#"SELECT max("Date") FROM "Revenue""#)
            .fetch_one(pool)
            .await
            .context("Failed to fetch latest revenue month")?;

        Ok(DataHealthSnapshot {
            market_counts,
            quoted_dates,
            missing_moving_averages: count_between(
                MISSING_MOVING_AVERAGES_SQL,
                from,
                to,
                "missing moving averages",
            )
            .await?,
            change_range_mismatches: count_between(
                CHANGE_RANGE_MISMATCHES_SQL,
                from,
                to,
                "change range mismatches",
            )
            .await?,
            misdated_rows: count_between(MISDATED_ROWS_SQL, from, to, "misdated rows").await?,
            derived_tables,
            stale_dividend_placeholders,
            dividend_total_mismatches,
            latest_revenue_month,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每一條量測 SQL 都要能在測試庫（由 `etc/sql` 建立）上執行。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn fetch_snapshot_runs_every_query() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 fetch_snapshot_runs_every_query：無資料庫連接");
            return;
        }

        let day = |d| NaiveDate::from_ymd_opt(2026, 4, d).expect("日期應合法");
        let snapshot = PgDataHealthRepository::new()
            .fetch_snapshot(day(1), day(24), day(30), 2026)
            .await
            .expect("量測應成功");

        assert_eq!(snapshot.derived_tables.len(), DERIVED_TABLES.len());
        assert!(snapshot.missing_moving_averages >= 0);
    }
}
