//! 通知項目目錄寫入 `notify_topic`（會員系統計畫 7.3）。
//!
//! 目錄的唯一來源是 [`crate::core::notify_topic::Topic`]；服務啟動時整份 upsert，stock_go 的設定頁讀表產生開關。
//! 目錄移除的項目不在這裡刪除：訂閱表以外鍵 `ON DELETE CASCADE` 參照它，刪除會連帶抹掉會員的選擇。

use anyhow::{Context, Result};
use sqlx::PgPool;

use crate::core::notify_topic::Topic;

/// 項目之間的排序間距；日後在中間插入項目時不必重排全部。
const SORT_STEP: i32 = 10;

/// 把整份目錄 upsert 到 `notify_topic`，回傳寫入的列數。
///
/// # Errors
///
/// 資料庫無法連線、`notify_topic` 不存在或沒有寫入權限時回傳錯誤。
pub async fn upsert_catalog(pool: &PgPool) -> Result<u64> {
    let capacity = Topic::ALL.len();
    let mut codes = Vec::with_capacity(capacity);
    let mut names = Vec::with_capacity(capacity);
    let mut descriptions = Vec::with_capacity(capacity);
    let mut categories = Vec::with_capacity(capacity);
    let mut audiences = Vec::with_capacity(capacity);
    let mut permissions = Vec::with_capacity(capacity);
    let mut root_only = Vec::with_capacity(capacity);
    let mut default_enabled = Vec::with_capacity(capacity);
    let mut sort_order = Vec::with_capacity(capacity);
    for (index, topic) in Topic::ALL.iter().enumerate() {
        let spec = topic.spec();
        codes.push(spec.code);
        names.push(spec.name);
        descriptions.push(spec.description);
        categories.push(spec.category.as_str());
        audiences.push(spec.audience.as_str());
        permissions.push(spec.required_permission);
        root_only.push(spec.root_only);
        default_enabled.push(spec.default_enabled);
        let position = i32::try_from(index).context("通知項目數量超出範圍")?;
        sort_order.push((position + 1) * SORT_STEP);
    }

    let result = sqlx::query(
        r#"
INSERT INTO notify_topic (code, name, description, category, audience, required_permission, root_only, default_enabled, sort_order)
SELECT * FROM UNNEST($1::varchar[], $2::varchar[], $3::text[], $4::varchar[], $5::varchar[], $6::varchar[], $7::bool[], $8::bool[], $9::int[])
ON CONFLICT (code) DO UPDATE SET
    name = EXCLUDED.name,
    description = EXCLUDED.description,
    category = EXCLUDED.category,
    audience = EXCLUDED.audience,
    required_permission = EXCLUDED.required_permission,
    root_only = EXCLUDED.root_only,
    default_enabled = EXCLUDED.default_enabled,
    sort_order = EXCLUDED.sort_order
"#,
    )
    .bind(&codes)
    .bind(&names)
    .bind(&descriptions)
    .bind(&categories)
    .bind(&audiences)
    .bind(&permissions)
    .bind(&root_only)
    .bind(&default_enabled)
    .bind(&sort_order)
    .execute(pool)
    .await
    .context("寫入 notify_topic 失敗")?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests;
