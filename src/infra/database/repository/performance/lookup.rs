//! CAGR 倉儲的查詢：個股各期間結果、樣本涵蓋統計、最新基準日與缺少某期間的基準日。

use anyhow::{Context, Result};
use chrono::NaiveDate;
use sqlx::Row;

use super::{SELECT_COLUMNS, cagr_column, rows_to_domain};
use crate::domain::performance::entity::{
    CagrCoverage, CagrMetric, CagrPeriod, StockCagr as DomainStockCagr,
};
use crate::infra::database;
use crate::infra::database::table::performance::stock_cagr::StockCagr as TableStockCagr;

/// 取得單一個股在指定基準日的所有期間結果（含資料不足者）。
///
/// 期間代碼在資料庫是字串，字典序（M3 < M6 < Y1 < Y10 < Y1H …）與時間長度
/// 不一致，故排序改在領域層以 [`CagrPeriod::months`] 進行。
pub(super) async fn fetch_by_symbol(
    date: NaiveDate,
    stock_symbol: &str,
) -> Result<Vec<DomainStockCagr>> {
    let sql = format!(
        r#"
SELECT {SELECT_COLUMNS}
FROM stock_cagr
WHERE date = $1 AND stock_symbol = $2
"#
    );

    let rows = sqlx::query_as::<_, TableStockCagr>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(date)
        .bind(stock_symbol)
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch stock_cagr in PgCagrRepository::fetch_by_symbol")?;

    let mut result = rows_to_domain(rows)?;
    result.sort_by_key(|item| item.period.months());

    Ok(result)
}

/// 取得指定基準日與期間的樣本涵蓋統計。
///
/// 五個計數以單一查詢的 `COUNT(*) FILTER` 完成，避免多次往返造成
/// 各計數取自不同時點的資料而彼此矛盾。
pub(super) async fn fetch_coverage(
    date: NaiveDate,
    period: CagrPeriod,
    metric: CagrMetric,
) -> Result<CagrCoverage> {
    // 同 fetch_ranking：欄位名稱來自白名單字面值。
    let column = cagr_column(metric);
    let sql = format!(
        r#"
SELECT
COUNT(*) AS universe,
COUNT(*) FILTER (WHERE data_complete) AS counted,
COUNT(*) FILTER (WHERE NOT data_complete) AS incomplete,
COUNT(*) FILTER (WHERE has_anomaly) AS anomaly_flagged,
COUNT(*) FILTER (WHERE data_complete AND {column} > 0) AS positive
FROM stock_cagr
WHERE date = $1 AND period = $2
"#
    );

    let row = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(date)
        .bind(period.code())
        .fetch_one(database::get_connection())
        .await
        .context("Failed to fetch coverage in PgCagrRepository::fetch_coverage")?;

    Ok(CagrCoverage {
        universe: row.try_get("universe")?,
        counted: row.try_get("counted")?,
        incomplete: row.try_get("incomplete")?,
        anomaly_flagged: row.try_get("anomaly_flagged")?,
        positive: row.try_get("positive")?,
    })
}

/// 取得最新一個已完成計算的基準日。
pub(super) async fn fetch_latest_date() -> Result<Option<NaiveDate>> {
    let row: (Option<NaiveDate>,) = sqlx::query_as("SELECT MAX(date) FROM stock_cagr")
        .fetch_one(database::get_connection())
        .await
        .context("Failed to fetch latest date in PgCagrRepository::fetch_latest_date")?;

    Ok(row.0)
}

/// 取得還沒有指定期間任何一列結果的基準日（回填單一期間時用）。
pub(super) async fn fetch_dates_missing_period(period: CagrPeriod) -> Result<Vec<NaiveDate>> {
    // 以 NOT EXISTS 而非 LEFT JOIN：只要該日期出現過任何一列指定期間的
    // 結果就算已回填，不需要逐檔比對母體。
    let sql = r#"
        SELECT DISTINCT s.date
        FROM stock_cagr s
        WHERE NOT EXISTS (
            SELECT 1
            FROM stock_cagr t
            WHERE t.date = s.date AND t.period = $1
        )
        ORDER BY s.date
    "#;

    let rows: Vec<(NaiveDate,)> = sqlx::query_as(sql)
        .bind(period.code())
        .fetch_all(database::get_connection())
        .await
        .context(
            "Failed to fetch dates missing period in PgCagrRepository::fetch_dates_missing_period",
        )?;

    Ok(rows.into_iter().map(|(date,)| date).collect())
}
