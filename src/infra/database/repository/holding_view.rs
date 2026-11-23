//! 把 [`HoldingView`] 解析成會員編號清單，供持股 SQL 以 `member_id = ANY($n)` 篩選（會員系統計畫 7.6 N2）。

use anyhow::{Context, Result};

use crate::domain::portfolio::view::HoldingView;
use crate::infra::database;

/// 範圍內的會員編號：指定成員時只有他；全體成員時為該家庭未刪除的成員（含已停用，停用仍結算）。
///
/// # Errors
///
/// 查詢 `"Member"` 失敗時回傳錯誤。
pub async fn member_ids(view: &HoldingView) -> Result<Vec<i64>> {
    if let Some(member_id) = view.member_id() {
        return Ok(vec![member_id]);
    }
    sqlx::query_scalar(
        r#"SELECT "Id" FROM "Member" WHERE household_id = $1 AND deleted_at IS NULL ORDER BY "Id""#,
    )
    .bind(view.household_id())
    .fetch_all(database::get_connection())
    .await
    .with_context(|| format!("查詢家庭 {} 的成員失敗", view.household_id()))
}

#[cfg(test)]
mod tests;
