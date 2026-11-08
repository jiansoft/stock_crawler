use crate::domain::trace::entity::PriceTrace;
use crate::domain::trace::repository::TraceRepository;
use crate::infra::database;
use anyhow::{Context, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use sqlx::FromRow;

/// 基於 PostgreSQL 的價格追蹤倉儲實現 (PgTraceRepository)。
///
/// 負責從 `"trace"` 資料表中載入所有的價格監控區間設定。
pub struct PgTraceRepository;

impl PgTraceRepository {
    /// 建立新的 PgTraceRepository 實例。
    pub fn new() -> Self {
        PgTraceRepository
    }
}

impl Default for PgTraceRepository {
    fn default() -> Self {
        Self::new()
    }
}

/// 資料庫對應的內部資料列結構體。
#[derive(FromRow)]
struct TraceDbRow {
    stock_symbol: String,
    floor: Decimal,
    ceiling: Decimal,
}

impl From<TraceDbRow> for PriceTrace {
    fn from(row: TraceDbRow) -> Self {
        PriceTrace::new(row.stock_symbol, row.floor, row.ceiling)
    }
}

#[async_trait]
impl TraceRepository for PgTraceRepository {
    /// 取得所有進行監控的價格追蹤設定清單。
    async fn fetch_all(&self) -> Result<Vec<PriceTrace>> {
        let sql = r#"SELECT "stock_symbol", "floor", "ceiling" FROM "trace";"#;

        let rows = sqlx::query_as::<_, TraceDbRow>(sql)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to query trace table from PostgreSQL")?;

        Ok(rows.into_iter().map(PriceTrace::from).collect())
    }
}

#[cfg(test)]
mod tests;
