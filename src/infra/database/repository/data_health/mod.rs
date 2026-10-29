//! 資料健康檢查的 PostgreSQL 量測實作。

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{Days, NaiveDate};

use crate::{
    domain::{
        health::{
            DataHealthRepository, DataHealthSnapshot, DerivedTableLatest, EtfNavDayCount,
            ForwardReturnGap, ValuationRatioZeroCount,
        },
        quote::entity::MarketTradedCount,
    },
    infra::database::{self, table::daily_quote},
};

#[cfg(test)]
mod market_return_tests;
mod sql;

// 檢查用 SQL 搬到子模組；倉儲實作留在這裡。
use self::sql::*;

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

/// 集保級距最新週、該週列數與千張大戶最新週 400 張空值檔數（[`HOLDER_LEVEL_HEALTH_SQL`]）。
async fn fetch_holder_level_health<'e>(
    executor: impl sqlx::PgExecutor<'e>,
) -> Result<(Option<NaiveDate>, i64, i64)> {
    sqlx::query_as(HOLDER_LEVEL_HEALTH_SQL)
        .fetch_one(executor)
        .await
        .context("Failed to fetch holder level health")
}

#[async_trait]
impl DataHealthRepository for PgDataHealthRepository {
    async fn fetch_snapshot(
        &self,
        baseline_from: NaiveDate,
        from: NaiveDate,
        to: NaiveDate,
        current_year: i32,
        profile_due: NaiveDate,
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

        let valuation_ratio_zeros =
            sqlx::query_as::<_, (NaiveDate, i32, i64, i64, i64)>(VALUATION_RATIO_ZEROS_SQL)
                .bind(from)
                .bind(to)
                .fetch_all(pool)
                .await
                .with_context(|| format!("Failed to count zero valuation ratios {from}~{to}"))?
                .into_iter()
                .map(
                    |(date, market_id, rows, pe_zero, pb_zero)| ValuationRatioZeroCount {
                        date,
                        market_id,
                        rows,
                        pe_zero,
                        pb_zero,
                    },
                )
                .collect();

        let forward_return_gaps = sqlx::query_as::<_, (NaiveDate, i64)>(FORWARD_RETURN_GAPS_SQL)
            .bind(FORWARD_RETURN_GAP_MIN_PERCENT)
            .fetch_all(pool)
            .await
            .context("Failed to find forward return gaps")?
            .into_iter()
            .map(|(date, rows)| ForwardReturnGap { date, rows })
            .collect();

        let drift_from = to - Days::new(MARKET_FORWARD_RETURN_DRIFT_DAYS);
        let market_forward_return_drift = sqlx::query_scalar(MARKET_FORWARD_RETURN_DRIFT_SQL)
            .bind(drift_from)
            .fetch_all(pool)
            .await
            .with_context(|| {
                format!("Failed to compare market forward returns since {drift_from}")
            })?;

        let etf_nav_counts = sqlx::query_as::<_, (NaiveDate, i64, i64)>(ETF_NAV_COUNTS_SQL)
            .bind(baseline_from)
            .bind(to)
            .fetch_all(pool)
            .await
            .with_context(|| format!("Failed to count ETF NAV rows {baseline_from}~{to}"))?
            .into_iter()
            .map(|(date, rows, missing_nav)| EtfNavDayCount {
                date,
                rows,
                missing_nav,
            })
            .collect();
        let stale_etf_profiles = sqlx::query_scalar(STALE_ETF_PROFILES_SQL)
            .bind(profile_due)
            .fetch_all(pool)
            .await
            .context("Failed to find stale ETF profiles")?;

        let latest_revenue_month = sqlx::query_scalar(r#"SELECT max("Date") FROM "Revenue""#)
            .fetch_one(pool)
            .await
            .context("Failed to fetch latest revenue month")?;
        let latest_holder_week =
            sqlx::query_scalar(r#"SELECT max("date") FROM holder_distribution"#)
                .fetch_one(pool)
                .await
                .context("Failed to fetch latest holder distribution week")?;
        let (latest_holder_level_week, holder_level_rows, holders_400_missing) =
            fetch_holder_level_health(pool).await?;
        let latest_insider_month =
            sqlx::query_scalar(r#"SELECT max("month") FROM insider_holding"#)
                .fetch_one(pool)
                .await
                .context("Failed to fetch latest insider holding month")?;

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
            chip_streak_missing: count_between(
                CHIP_STREAK_MISSING_SQL,
                from,
                to,
                "chip streak missing",
            )
            .await?,
            derived_tables,
            stale_dividend_placeholders,
            dividend_total_mismatches,
            latest_revenue_month,
            latest_holder_week,
            latest_holder_level_week,
            holder_level_rows,
            holders_400_missing,
            latest_insider_month,
            valuation_ratio_zeros,
            forward_return_gaps,
            market_forward_return_drift,
            etf_nav_counts,
            stale_etf_profiles,
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
            .fetch_snapshot(day(1), day(24), day(30), 2026, day(21))
            .await
            .expect("量測應成功");

        assert_eq!(snapshot.derived_tables.len(), DERIVED_TABLES.len());
        assert!(snapshot.missing_moving_averages >= 0);
        assert!(snapshot.chip_streak_missing >= 0);
    }

    /// ETF 淨值依日期計數；基本資料停更或上市已久仍沒有基本資料的才列出。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn etf_queries_count_nav_rows_and_find_stale_profiles() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 etf_queries_count_nav_rows_and_find_stale_profiles：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        let mut tx = pool.begin().await.expect("開交易應成功");
        // 交易內清空，測完 rollback，不影響其他測試的資料。
        for sql in [
            "DELETE FROM etf_snapshot",
            "DELETE FROM etf_profile",
            "DELETE FROM etf_daily_nav",
        ] {
            sqlx::query(sql)
                .execute(&mut *tx)
                .await
                .expect("清空應成功");
        }
        let day = |d| NaiveDate::from_ymd_opt(2021, 3, d).expect("日期應合法");
        for (symbol, date, nav) in [
            ("79971", day(1), Some(10.0)),
            ("79972", day(1), None),
            ("79971", day(2), Some(10.1)),
            ("79973", day(9), Some(9.9)),
        ] {
            sqlx::query(
                "INSERT INTO etf_daily_nav (date, stock_symbol, estimated_nav, nav) VALUES ($1, $2, $3, $3)",
            )
            .bind(date)
            .bind(symbol)
            .bind(nav)
            .execute(&mut *tx)
            .await
            .expect("寫入淨值應成功");
        }
        // 79971 剛更新、79972 停更、79973 剛上市還沒有基本資料（不列）、79974 上市已久沒有基本資料。
        for (symbol, updated) in [("79971", "2021-03-09"), ("79972", "2021-03-01")] {
            sqlx::query("INSERT INTO etf_profile (stock_symbol, updated_at) VALUES ($1, $2::date)")
                .bind(symbol)
                .bind(updated)
                .execute(&mut *tx)
                .await
                .expect("寫入基本資料應成功");
        }
        sqlx::query(
            "INSERT INTO etf_daily_nav (date, stock_symbol, nav) VALUES ('2021-02-01', '79974', 10)",
        )
        .execute(&mut *tx)
        .await
        .expect("寫入淨值應成功");
        for symbol in ["79971", "79972", "79973", "79974"] {
            sqlx::query(
                "INSERT INTO etf_snapshot (stock_symbol, name, market_id) VALUES ($1, $1, 2)",
            )
            .bind(symbol)
            .execute(&mut *tx)
            .await
            .expect("寫入快照應成功");
        }

        let counts: Vec<(NaiveDate, i64, i64)> = sqlx::query_as(ETF_NAV_COUNTS_SQL)
            .bind(day(1))
            .bind(day(9))
            .fetch_all(&mut *tx)
            .await
            .expect("計數應成功");
        let stale: Vec<String> = sqlx::query_scalar(STALE_ETF_PROFILES_SQL)
            .bind(day(5))
            .fetch_all(&mut *tx)
            .await
            .expect("查詢應成功");
        tx.rollback().await.expect("rollback 應成功");

        assert_eq!(counts, [(day(1), 2, 1), (day(2), 1, 0), (day(9), 1, 0)]);
        assert_eq!(stale, ["79972", "79974"]);
    }

    /// 短天期缺漏占比達門檻的基準日要列出；零星缺值與最早有 5 日報酬之前的基準日不列入。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn forward_return_gaps_flag_whole_day_holes_only() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 forward_return_gaps_flag_whole_day_holes_only：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        let mut tx = pool.begin().await.expect("開交易應成功");
        // 交易內清空，測完 rollback，不影響其他測試的資料。
        sqlx::query("DELETE FROM daily_forward_return")
            .execute(&mut *tx)
            .await
            .expect("清空應成功");
        let insert = r#"
INSERT INTO daily_forward_return (date, stock_symbol, return_5d, return_20d, return_60d)
SELECT $1::date, $4 || n, $2, 1.0, 1.0 FROM generate_series(1, $3) AS n
"#;
        let day = |d| NaiveDate::from_ymd_opt(2021, 3, d).expect("日期應合法");
        for (date, return_5d, rows, prefix) in [
            // 最早有 5 日報酬之前、只有長天期的基準日：不列入。
            (day(1), None, 150, "G"),
            (day(2), Some(1.0), 150, "G"),
            // 整天缺 5 日報酬：列出。
            (day(3), None, 120, "G"),
            // 零星缺值（3%）：不列入。
            (day(4), None, 3, "G"),
            (day(4), Some(1.0), 100, "H"),
            // 不常成交的債券 ETF（13%）：不列入。
            (day(5), None, 15, "G"),
            (day(5), Some(1.0), 100, "H"),
            // 一半缺漏：列出缺漏的列數。
            (day(8), None, 60, "G"),
            (day(8), Some(1.0), 60, "H"),
        ] {
            sqlx::query(insert)
                .bind(date)
                .bind(return_5d)
                .bind(rows)
                .bind(prefix)
                .execute(&mut *tx)
                .await
                .expect("寫入應成功");
        }

        let gaps: Vec<(NaiveDate, i64)> = sqlx::query_as(FORWARD_RETURN_GAPS_SQL)
            .bind(FORWARD_RETURN_GAP_MIN_PERCENT)
            .fetch_all(&mut *tx)
            .await
            .expect("查詢應成功");
        tx.rollback().await.expect("rollback 應成功");

        assert_eq!(gaps, [(day(3), 120), (day(8), 60)]);
    }

    /// 集保級距只量最新一週的列數；400 張空值只看千張大戶最新一週，舊週的空值不算。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn holder_level_health_counts_the_latest_week_only() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 holder_level_health_counts_the_latest_week_only：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        let mut tx = pool.begin().await.expect("開交易應成功");
        // 交易內清空，測完 rollback，不影響其他測試的資料。
        for sql in [
            "DELETE FROM holder_distribution_level",
            "DELETE FROM holder_distribution",
        ] {
            sqlx::query(sql)
                .execute(&mut *tx)
                .await
                .expect("清空應成功");
        }
        let empty = fetch_holder_level_health(&mut *tx)
            .await
            .expect("空表查詢應成功");
        assert_eq!(empty, (None, 0, 0));

        let week = |d| NaiveDate::from_ymd_opt(2021, 3, d).expect("日期應合法");
        // 舊週 2 檔、最新週 3 檔各 17 列。
        for (date, symbols) in [(week(5), 2), (week(12), 3)] {
            sqlx::query(
                r#"
INSERT INTO holder_distribution_level ("date", stock_symbol, level)
SELECT $1, 'L' || s, l FROM generate_series(1, $2) AS s, generate_series(1, 17) AS l
"#,
            )
            .bind(date)
            .bind(symbols)
            .execute(&mut *tx)
            .await
            .expect("寫入級距應成功");
        }
        // 舊週全部空值；最新週 1 檔空值、1 檔有值。
        for (date, symbol, holders_400) in [
            (week(5), "H1", None),
            (week(5), "H2", None),
            (week(12), "H1", Some(12_i64)),
            (week(12), "H2", None),
        ] {
            sqlx::query(
                r#"INSERT INTO holder_distribution ("date", stock_symbol, holders_400) VALUES ($1, $2, $3)"#,
            )
            .bind(date)
            .bind(symbol)
            .bind(holders_400)
            .execute(&mut *tx)
            .await
            .expect("寫入千張大戶應成功");
        }

        let health = fetch_holder_level_health(&mut *tx)
            .await
            .expect("查詢應成功");
        tx.rollback().await.expect("rollback 應成功");

        assert_eq!(health, (Some(week(12)), 51, 1));
    }
}
