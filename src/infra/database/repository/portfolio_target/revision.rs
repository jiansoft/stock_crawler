//! 目標配置的歷史版本（`portfolio_target_revision`）：儲存前留存覆蓋前的整組、列出與取得版本。

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgConnection};

use super::PgPortfolioTargetRepository;
use crate::domain::portfolio::target::{
    MAX_TARGET_REVISIONS, PortfolioTargetRevision, TargetDimension,
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
    saved_time: DateTime<Utc>,
    targets: String,
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
    member_id: &str,
    dimension: TargetDimension,
    new: &[(String, f64)],
) -> Result<()> {
    let configured: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM portfolio_target_dimension WHERE member_id=$1 AND dimension=$2)",
    )
    .bind(member_id)
    .bind(dimension.as_str())
    .fetch_one(&mut *conn)
    .await?;
    if !configured {
        return Ok(());
    }
    let current: Vec<(String, Decimal)> = sqlx::query_as(
        r#"SELECT key, target_percent FROM portfolio_target
           WHERE member_id = $1 AND dimension = $2
           ORDER BY target_percent DESC, key ASC"#,
    )
    .bind(member_id)
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
        "INSERT INTO portfolio_target_revision (member_id, dimension, targets) VALUES ($1, $2, $3::jsonb)",
    )
    .bind(member_id)
    .bind(dimension.as_str())
    .bind(serde_json::to_string(&items)?)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        r#"DELETE FROM portfolio_target_revision
           WHERE member_id = $1 AND dimension = $2
             AND id NOT IN (
                 SELECT id FROM portfolio_target_revision
                 WHERE member_id = $1 AND dimension = $2
                 ORDER BY saved_time DESC, id DESC
                 LIMIT $3
             )"#,
    )
    .bind(member_id)
    .bind(dimension.as_str())
    .bind(MAX_TARGET_REVISIONS)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

impl PgPortfolioTargetRepository {
    /// 列出某會員某維度的歷史版本，由新到舊（最多 [`MAX_TARGET_REVISIONS`] 版）。
    pub async fn list_revisions(
        &self,
        member_id: &str,
        dimension: TargetDimension,
    ) -> Result<Vec<PortfolioTargetRevision>> {
        let rows: Vec<RevisionRow> = sqlx::query_as(
            r#"SELECT id, dimension, saved_time, targets::text AS targets
               FROM portfolio_target_revision
               WHERE member_id = $1 AND dimension = $2
               ORDER BY saved_time DESC, id DESC"#,
        )
        .bind(member_id)
        .bind(dimension.as_str())
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(PortfolioTargetRevision::try_from)
            .collect()
    }

    /// 取得某會員的一個歷史版本；不存在或不屬於這個會員時回 `None`。
    pub async fn get_revision(
        &self,
        member_id: &str,
        id: i64,
    ) -> Result<Option<PortfolioTargetRevision>> {
        let row: Option<RevisionRow> = sqlx::query_as(
            r#"SELECT id, dimension, saved_time, targets::text AS targets
               FROM portfolio_target_revision
               WHERE member_id = $1 AND id = $2"#,
        )
        .bind(member_id)
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        row.map(PortfolioTargetRevision::try_from).transpose()
    }
}

#[cfg(test)]
mod tests;
