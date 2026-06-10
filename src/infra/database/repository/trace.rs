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
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[tokio::test]
    async fn test_pg_trace_repository_contract() {
        // 這是一個單元合約測試佔位符，如果沒有資料庫連接則跳過
        dotenv::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 PgTraceRepository DB 整合測試：無資料庫連接");
            return;
        }

        let repo = PgTraceRepository::new();
        let test_symbol = "__TEST_TRACE__";

        // 1. 清理潛在的殘留測試數據
        sqlx::query("DELETE FROM \"trace\" WHERE stock_symbol = $1")
            .bind(test_symbol)
            .execute(database::get_connection())
            .await
            .ok();

        // 2. 寫入一筆測試資料
        sqlx::query("INSERT INTO \"trace\" (stock_symbol, floor, ceiling) VALUES ($1, $2, $3)")
            .bind(test_symbol)
            .bind(dec!(100.5))
            .bind(dec!(200.5))
            .execute(database::get_connection())
            .await
            .unwrap();

        // 3. 呼叫倉儲的 fetch_all 驗證是否能載入該測試資料
        let traces = repo.fetch_all().await.unwrap();
        let found = traces.iter().find(|t| t.stock_symbol == test_symbol);
        assert!(found.is_some());
        let found_trace = found.unwrap();
        assert_eq!(found_trace.floor, dec!(100.5));
        assert_eq!(found_trace.ceiling, dec!(200.5));

        // 4. 清理測試數據
        sqlx::query("DELETE FROM \"trace\" WHERE stock_symbol = $1")
            .bind(test_symbol)
            .execute(database::get_connection())
            .await
            .unwrap();
    }
}
