//! 批次讀取可見會員名稱；每次組通知重新查詢，不跨事件快取，改名後下一次通知即生效。

use std::collections::BTreeMap;

use anyhow::{Context, Result};

use crate::domain::portfolio::view::HoldingView;
use crate::infra::database;

/// 載入同家庭、範圍內未刪除會員的名稱，名稱保持原文，由各訊息的 Markdown 邊界跳脫。
///
/// # Errors
///
/// 資料庫查詢失敗時回傳錯誤，不以跨家庭名稱或過期快取代替。
pub async fn fetch_names(view: &HoldingView) -> Result<BTreeMap<i64, String>> {
    fetch_names_from(database::get_connection(), view).await
}

/// 可注入交易的查詢本體，讓整合測試建立資料後完整回滾。
pub(crate) async fn fetch_names_from<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    view: &HoldingView,
) -> Result<BTreeMap<i64, String>> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        r#"SELECT "Id", "Name" FROM "Member"
           WHERE household_id = $1 AND deleted_at IS NULL
             AND ($2::bigint IS NULL OR "Id" = $2)
           ORDER BY "Id""#,
    )
    .bind(view.household_id())
    .bind(view.member_id())
    .fetch_all(executor)
    .await
    .context("讀取通知會員名稱失敗")?;
    Ok(rows.into_iter().collect())
}

#[cfg(test)]
mod tests;
