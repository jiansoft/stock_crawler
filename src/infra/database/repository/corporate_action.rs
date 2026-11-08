//! 公司行動（分割／減資）的 PostgreSQL 倉儲實作。

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::{Row, postgres::PgRow};

use crate::domain::performance::entity::{CorporateAction, CorporateActionType};
use crate::domain::performance::repository::CorporateActionRepository;
use crate::infra::database;

/// 對應資料表 `public.corporate_action`，主鍵為 `(stock_symbol, effective_date)`。
#[derive(Debug, Clone, Copy, Default)]
pub struct PgCorporateActionRepository;

impl PgCorporateActionRepository {
    /// 建立實例。
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl CorporateActionRepository for PgCorporateActionRepository {
    async fn save(&self, action: &CorporateAction) -> Result<u64> {
        // 同一 (代號, 生效日) 重複登錄視為修正：比例打錯時直接重送即可，
        // 不必先刪除。
        let sql = r#"
            INSERT INTO corporate_action (stock_symbol, effective_date, action_type, share_ratio, note)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (stock_symbol, effective_date) DO UPDATE SET
                action_type = excluded.action_type,
                share_ratio = excluded.share_ratio,
                note = excluded.note,
                updated_time = now()
        "#;

        // 型別由呼叫端明確指定，不再由比例反推：減資退還股款的比例可能大於 1
        // （參考價已扣掉退還的現金），反推會把它誤存成 split。
        let action_type = action.action_type.as_str();

        let result = sqlx::query(sql)
            .bind(&action.stock_symbol)
            .bind(action.effective_date)
            .bind(action_type)
            .bind(action.share_ratio)
            .bind(&action.note)
            .execute(database::get_connection())
            .await
            .context("Failed to save corporate action")?;

        Ok(result.rows_affected())
    }

    async fn fetch_by_symbol(&self, stock_symbol: &str) -> Result<Vec<CorporateAction>> {
        let sql = r#"
            SELECT stock_symbol, effective_date, action_type, share_ratio, note
            FROM corporate_action
            WHERE stock_symbol = $1
            ORDER BY effective_date
        "#;

        let rows = sqlx::query(sql)
            .bind(stock_symbol)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch corporate actions by symbol")?;

        rows.iter().map(corporate_action_from_row).collect()
    }

    async fn fetch_by_effective_date(&self, date: NaiveDate) -> Result<Vec<CorporateAction>> {
        let sql = r#"
            SELECT stock_symbol, effective_date, action_type, share_ratio, note
            FROM corporate_action
            WHERE effective_date = $1
            ORDER BY stock_symbol
        "#;

        let rows = sqlx::query(sql)
            .bind(date)
            .fetch_all(database::get_connection())
            .await
            .with_context(|| format!("Failed to fetch corporate actions effective on {date}"))?;

        rows.iter().map(corporate_action_from_row).collect()
    }
}

/// 將 `corporate_action` 資料列轉成領域實體。
fn corporate_action_from_row(row: &PgRow) -> Result<CorporateAction> {
    let share_ratio = row.try_get::<Decimal, _>("share_ratio")?;
    // 舊資料或人工以 SQL 寫入的列可能有無法辨識的 action_type，
    // 此時退回以比例推斷，至少不會讓整批查詢失敗。
    let action_type = CorporateActionType::from_db_value(row.try_get::<&str, _>("action_type")?)
        .unwrap_or_else(|| CorporateActionType::infer_from_ratio(share_ratio));

    Ok(CorporateAction {
        stock_symbol: row.try_get::<String, _>("stock_symbol")?,
        effective_date: row.try_get::<NaiveDate, _>("effective_date")?,
        action_type,
        share_ratio,
        note: row.try_get::<String, _>("note")?,
    })
}

#[cfg(test)]
mod tests;
