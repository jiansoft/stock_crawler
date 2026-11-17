use crate::domain::portfolio::view::HoldingView;
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
    member_id: i64,
    stock_symbol: String,
    floor: Decimal,
    ceiling: Decimal,
}

impl From<TraceDbRow> for PriceTrace {
    fn from(row: TraceDbRow) -> Self {
        PriceTrace::new(row.member_id, row.stock_symbol, row.floor, row.ceiling)
    }
}

#[async_trait]
impl TraceRepository for PgTraceRepository {
    /// 取得所有會員進行監控的價格追蹤設定清單（同一檔可能有多位會員各一筆）。
    async fn fetch_all(&self) -> Result<Vec<PriceTrace>> {
        let sql = r#"SELECT "member_id", "stock_symbol", "floor", "ceiling" FROM "trace" ORDER BY "member_id", "stock_symbol";"#;

        let rows = sqlx::query_as::<_, TraceDbRow>(sql)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to query trace table from PostgreSQL")?;

        Ok(rows.into_iter().map(PriceTrace::from).collect())
    }
}

/// 批次載入可見範圍內的追蹤擁有者名稱，同時以家庭邊界排除外部會員與已刪除會員。
pub async fn fetch_visible_owners(view: HoldingView) -> Result<Vec<(i64, String)>> {
    fetch_visible_owners_from(database::get_connection(), view).await
}

/// 可注入交易的查詢本體，讓家庭邊界測試在交易內建立會員並完整回滾。
async fn fetch_visible_owners_from<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    view: HoldingView,
) -> Result<Vec<(i64, String)>> {
    sqlx::query_as(
        r#"SELECT "Id", "Name" FROM "Member"
           WHERE household_id = $1 AND deleted_at IS NULL
             AND ($2::bigint IS NULL OR "Id" = $2)
           ORDER BY "Id""#,
    )
    .bind(view.household_id())
    .bind(view.member_id())
    .fetch_all(executor)
    .await
    .context("讀取價格追蹤擁有者失敗")
}

#[cfg(test)]
mod tests;
