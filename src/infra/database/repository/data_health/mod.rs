//! 資料健康檢查的 PostgreSQL 量測實作。

use jiff_sqlx::ToSqlx;

use crate::infra::database::jiff_types::from_wire;
use anyhow::{Context, Result};
use async_trait::async_trait;
use jiff::ToSpan;
use jiff::civil::Date;

use crate::{
    domain::{
        health::{
            DataHealthRepository, DataHealthSnapshot, DerivedTableLatest, EtfNavDayCount,
            ForwardReturnGap, StatementStats, ValuationRatioZeroCount,
        },
        quote::entity::MarketTradedCount,
    },
    infra::database::{self, table::daily_quote},
};

#[cfg(test)]
mod market_return_tests;
mod sql;
mod statements;
#[cfg(test)]
mod statements_tests;
#[cfg(test)]
mod tests;

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
async fn count_between(sql: &'static str, from: Date, to: Date, label: &str) -> Result<i64> {
    sqlx::query_scalar(sql)
        .bind(from.to_sqlx())
        .bind(to.to_sqlx())
        .fetch_one(database::get_connection())
        .await
        .with_context(|| format!("Failed to count {label} {from}~{to}"))
}

/// 集保級距最新週、該週列數與千張大戶最新週 400 張空值檔數（[`HOLDER_LEVEL_HEALTH_SQL`]）。
async fn fetch_holder_level_health<'e>(
    executor: impl sqlx::PgExecutor<'e>,
) -> Result<(Option<Date>, i64, i64)> {
    let (week, rows, missing): (Option<jiff_sqlx::Date>, i64, i64) =
        sqlx::query_as(HOLDER_LEVEL_HEALTH_SQL)
            .fetch_one(executor)
            .await
            .context("Failed to fetch holder level health")?;
    Ok((week.map(jiff_sqlx::Date::to_jiff), rows, missing))
}

#[async_trait]
impl DataHealthRepository for PgDataHealthRepository {
    async fn fetch_statement_stats(&self) -> Result<Option<StatementStats>> {
        statements::fetch_statement_stats().await
    }

    async fn fetch_snapshot(
        &self,
        baseline_from: Date,
        from: Date,
        to: Date,
        current_year: i32,
        profile_due: Date,
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

        let quoted_dates = sqlx::query_scalar::<_, jiff_sqlx::Date>(
            r#"SELECT DISTINCT "Date" FROM "DailyQuotes" WHERE "Date" BETWEEN $1 AND $2 ORDER BY 1"#,
        )
        .bind(from.to_sqlx())
        .bind(to.to_sqlx())
        .fetch_all(pool)
        .await
        .with_context(|| format!("Failed to fetch quoted dates {from}~{to}"))?
        .into_iter()
        .map(from_wire)
        .collect();

        let mut derived_tables = Vec::with_capacity(DERIVED_TABLES.len());
        for (table, sql) in DERIVED_TABLES {
            let latest: Option<Date> = sqlx::query_scalar::<_, Option<jiff_sqlx::Date>>(sql)
                .fetch_one(pool)
                .await
                .with_context(|| format!("Failed to fetch latest date of {table}"))?
                .map(jiff_sqlx::Date::to_jiff);
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
            sqlx::query_as::<_, (jiff_sqlx::Date, i32, i64, i64, i64)>(VALUATION_RATIO_ZEROS_SQL)
                .bind(from.to_sqlx())
                .bind(to.to_sqlx())
                .fetch_all(pool)
                .await
                .with_context(|| format!("Failed to count zero valuation ratios {from}~{to}"))?
                .into_iter()
                .map(
                    |(date, market_id, rows, pe_zero, pb_zero)| ValuationRatioZeroCount {
                        date: date.to_jiff(),
                        market_id,
                        rows,
                        pe_zero,
                        pb_zero,
                    },
                )
                .collect();

        let forward_return_gaps =
            sqlx::query_as::<_, (jiff_sqlx::Date, i64)>(FORWARD_RETURN_GAPS_SQL)
                .bind(FORWARD_RETURN_GAP_MIN_PERCENT)
                .fetch_all(pool)
                .await
                .context("Failed to find forward return gaps")?
                .into_iter()
                .map(|(date, rows)| ForwardReturnGap {
                    date: date.to_jiff(),
                    rows,
                })
                .collect();

        let drift_from = to - MARKET_FORWARD_RETURN_DRIFT_DAYS.days();
        let market_forward_return_drift =
            sqlx::query_scalar::<_, jiff_sqlx::Date>(MARKET_FORWARD_RETURN_DRIFT_SQL)
                .bind(drift_from.to_sqlx())
                .fetch_all(pool)
                .await
                .with_context(|| {
                    format!("Failed to compare market forward returns since {drift_from}")
                })?
                .into_iter()
                .map(from_wire)
                .collect();

        let etf_nav_counts = sqlx::query_as::<_, (jiff_sqlx::Date, i64, i64)>(ETF_NAV_COUNTS_SQL)
            .bind(baseline_from.to_sqlx())
            .bind(to.to_sqlx())
            .fetch_all(pool)
            .await
            .with_context(|| format!("Failed to count ETF NAV rows {baseline_from}~{to}"))?
            .into_iter()
            .map(|(date, rows, missing_nav)| EtfNavDayCount {
                date: date.to_jiff(),
                rows,
                missing_nav,
            })
            .collect();
        let stale_etf_profiles = sqlx::query_scalar(STALE_ETF_PROFILES_SQL)
            .bind(profile_due.to_sqlx())
            .fetch_all(pool)
            .await
            .context("Failed to find stale ETF profiles")?;

        let latest_revenue_month = sqlx::query_scalar(r#"SELECT max("Date") FROM "Revenue""#)
            .fetch_one(pool)
            .await
            .context("Failed to fetch latest revenue month")?;
        let latest_holder_week = sqlx::query_scalar::<_, Option<jiff_sqlx::Date>>(
            r#"SELECT max("date") FROM holder_distribution"#,
        )
        .fetch_one(pool)
        .await
        .context("Failed to fetch latest holder distribution week")?
        .map(jiff_sqlx::Date::to_jiff);
        let (latest_holder_level_week, holder_level_rows, holders_400_missing) =
            fetch_holder_level_health(pool).await?;
        let latest_insider_month = sqlx::query_scalar::<_, Option<jiff_sqlx::Date>>(
            r#"SELECT max("month") FROM insider_holding"#,
        )
        .fetch_one(pool)
        .await
        .context("Failed to fetch latest insider holding month")?
        .map(jiff_sqlx::Date::to_jiff);

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
