//! ETF 相對 0050 的代理基準報酬差異；保留資料日與有效樣本數。
use anyhow::Result;
use chrono::NaiveDate;
use sqlx::FromRow;

use crate::domain::etf::tracking::TrackingStats;

#[derive(FromRow)]
pub(crate) struct TrackingRow {
    pub target_benchmark: String,
    pub calc_date: Option<NaiveDate>,
    pub samples: Option<i32>,
    pub tracking_error_percent: Option<f64>,
    pub tracking_difference_percent: Option<f64>,
}

pub(crate) async fn find(symbol: &str) -> Result<Option<TrackingRow>> {
    Ok(sqlx::query_as(
        "SELECT s.benchmark AS target_benchmark, t.calc_date, t.samples, t.tracking_error_percent, t.tracking_difference_percent FROM etf_snapshot s LEFT JOIN etf_tracking_error t ON t.stock_symbol=s.stock_symbol AND t.benchmark_symbol='0050' WHERE s.stock_symbol=$1",
    ).bind(symbol).fetch_optional(crate::infra::database::get_connection()).await?)
}

pub(crate) async fn save(rows: &[(String, NaiveDate, TrackingStats)]) -> Result<()> {
    let mut transaction = crate::infra::database::get_tx().await?;
    for (symbol, date, stats) in rows {
        sqlx::query("INSERT INTO etf_tracking_error (stock_symbol,benchmark_symbol,calc_date,samples,tracking_error_percent,tracking_difference_percent) VALUES ($1,'0050',$2,$3,$4,$5) ON CONFLICT (stock_symbol,benchmark_symbol) DO UPDATE SET calc_date=EXCLUDED.calc_date,samples=EXCLUDED.samples,tracking_error_percent=EXCLUDED.tracking_error_percent,tracking_difference_percent=EXCLUDED.tracking_difference_percent")
            .bind(symbol).bind(date).bind(stats.samples as i32).bind(stats.tracking_error).bind(stats.tracking_difference)
            .execute(&mut *transaction).await?;
    }
    transaction.commit().await?;
    Ok(())
}
