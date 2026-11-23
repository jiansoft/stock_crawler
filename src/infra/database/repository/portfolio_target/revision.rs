//! 目標配置的歷史版本（`portfolio_target_revision`）：儲存前留存覆蓋前的整組、列出與取得版本。

use anyhow::{Context, Result};
use jiff::Timestamp;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgConnection};

use super::PgPortfolioTargetRepository;
use crate::domain::portfolio::target::{
    ActorSource, MAX_TARGET_REVISIONS, PortfolioTargetRevision, TargetActor, TargetDimension,
    TargetOwner,
};

/// 存在 `targets`（jsonb）裡的一個項目。
#[derive(Debug, Serialize, Deserialize)]
struct RevisionItem {
    key: String,
    target_percent: f64,
}

/// `portfolio_target_revision` 的資料列；`targets` 以文字讀出再解析。
#[derive(FromRow)]
struct RevisionRow {
    id: i64,
    dimension: String,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    saved_time: Timestamp,
    targets: String,
    actor_source: String,
    actor_ref: String,
}

impl TryFrom<RevisionRow> for PortfolioTargetRevision {
    type Error = anyhow::Error;

    fn try_from(row: RevisionRow) -> Result<Self> {
        let dimension = TargetDimension::parse(&row.dimension).context("未知的配置維度")?;
        let items: Vec<RevisionItem> =
            serde_json::from_str(&row.targets).context("目標配置版本內容格式錯誤")?;
        Ok(Self {
            id: row.id,
            dimension,
            saved_time: row.saved_time,
            targets: items
                .into_iter()
                .map(|item| (item.key, item.target_percent))
                .collect(),
            // 資料庫沒有限制可寫入的值；不認得的來源當作未標示，不讓整筆讀取失敗
            actor: TargetActor {
                source: ActorSource::parse(&row.actor_source).unwrap_or_default(),
                actor_ref: row.actor_ref,
            },
        })
    }
}

/// 新的一組和目前的一組是否相同（比對到資料庫存的兩位小數，不看順序）。
pub(super) fn unchanged(current: &[(String, Decimal)], new: &[(String, f64)]) -> bool {
    let mut current: Vec<(&str, Decimal)> = current
        .iter()
        .map(|(key, percent)| (key.as_str(), percent.round_dp(2)))
        .collect();
    let mut new: Vec<(&str, Decimal)> = new
        .iter()
        .map(|(key, percent)| {
            let percent = Decimal::from_f64_retain(*percent).unwrap_or_default();
            (key.as_str(), percent.round_dp(2))
        })
        .collect();
    current.sort_unstable();
    new.sort_unstable();
    current == new
}

/// 在 `save_targets` 的交易內、覆蓋之前呼叫：該維度原本已設定（含全現金）且內容有變時，
/// 把目前的整組記成一版，並只保留最近 [`MAX_TARGET_REVISIONS`] 版。從未設定過的維度不記。
pub(super) async fn record_before_save(
    conn: &mut PgConnection,
    owner: &TargetOwner,
    dimension: TargetDimension,
    new: &[(String, f64)],
    actor: &TargetActor,
) -> Result<()> {
    let configured: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM portfolio_target_dimension WHERE household_id=$1 AND member_id=$2 AND dimension=$3)",
    )
    .bind(owner.household_id)
    .bind(&owner.member_id)
    .bind(dimension.as_str())
    .fetch_one(&mut *conn)
    .await?;
    if !configured {
        return Ok(());
    }
    let current: Vec<(String, Decimal)> = sqlx::query_as(
        r#"SELECT key, target_percent FROM portfolio_target
           WHERE household_id = $1 AND member_id = $2 AND dimension = $3
           ORDER BY target_percent DESC, key ASC"#,
    )
    .bind(owner.household_id)
    .bind(&owner.member_id)
    .bind(dimension.as_str())
    .fetch_all(&mut *conn)
    .await?;
    if unchanged(&current, new) {
        return Ok(());
    }

    let items: Vec<RevisionItem> = current
        .into_iter()
        .map(|(key, percent)| RevisionItem {
            key,
            target_percent: percent.to_f64().unwrap_or_default(),
        })
        .collect();
    sqlx::query(
        r#"INSERT INTO portfolio_target_revision
               (household_id, member_id, dimension, targets, actor_source, actor_ref)
           VALUES ($1, $2, $3, $4::jsonb, $5, $6)"#,
    )
    .bind(owner.household_id)
    .bind(&owner.member_id)
    .bind(dimension.as_str())
    .bind(serde_json::to_string(&items)?)
    .bind(actor.source.as_str())
    .bind(&actor.actor_ref)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        r#"DELETE FROM portfolio_target_revision
           WHERE household_id = $1 AND member_id = $2 AND dimension = $3
             AND id NOT IN (
                 SELECT id FROM portfolio_target_revision
                 WHERE household_id = $1 AND member_id = $2 AND dimension = $3
                 ORDER BY saved_time DESC, id DESC
                 LIMIT $4
             )"#,
    )
    .bind(owner.household_id)
    .bind(&owner.member_id)
    .bind(dimension.as_str())
    .bind(MAX_TARGET_REVISIONS)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

impl PgPortfolioTargetRepository {
    /// 列出某擁有者某維度的歷史版本，由新到舊（最多 [`MAX_TARGET_REVISIONS`] 版）。
    pub async fn list_revisions(
        &self,
        owner: &TargetOwner,
        dimension: TargetDimension,
    ) -> Result<Vec<PortfolioTargetRevision>> {
        let rows: Vec<RevisionRow> = sqlx::query_as(
            r#"SELECT id, dimension, saved_time, targets::text AS targets, actor_source, actor_ref
               FROM portfolio_target_revision
               WHERE household_id = $1 AND member_id = $2 AND dimension = $3
               ORDER BY saved_time DESC, id DESC"#,
        )
        .bind(owner.household_id)
        .bind(&owner.member_id)
        .bind(dimension.as_str())
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(PortfolioTargetRevision::try_from)
            .collect()
    }

    /// 取得某擁有者的一個歷史版本；不存在或不屬於這個家庭與會員時回 `None`。
    pub async fn get_revision(
        &self,
        owner: &TargetOwner,
        id: i64,
    ) -> Result<Option<PortfolioTargetRevision>> {
        let row: Option<RevisionRow> = sqlx::query_as(
            r#"SELECT id, dimension, saved_time, targets::text AS targets, actor_source, actor_ref
               FROM portfolio_target_revision
               WHERE household_id = $1 AND member_id = $2 AND id = $3"#,
        )
        .bind(owner.household_id)
        .bind(&owner.member_id)
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        row.map(PortfolioTargetRevision::try_from).transpose()
    }
}

#[cfg(test)]
mod tests;
