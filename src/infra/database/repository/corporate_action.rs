//! 公司行動（分割／減資）的 PostgreSQL 倉儲實作。

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::Row;

use crate::domain::performance::entity::CorporateAction;
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

        // 比例小於 1 是減資或反向分割，大於 1 是分割；型別由比例推得，
        // 讓登錄者少填一個容易與比例矛盾的欄位。
        let action_type = if action.share_ratio < Decimal::ONE {
            "capital_reduction"
        } else {
            "split"
        };

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
            SELECT stock_symbol, effective_date, share_ratio, note
            FROM corporate_action
            WHERE stock_symbol = $1
            ORDER BY effective_date
        "#;

        let rows = sqlx::query(sql)
            .bind(stock_symbol)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch corporate actions by symbol")?;

        rows.into_iter()
            .map(|row| {
                Ok(CorporateAction {
                    stock_symbol: row.try_get::<String, _>("stock_symbol")?,
                    effective_date: row.try_get::<NaiveDate, _>("effective_date")?,
                    share_ratio: row.try_get::<Decimal, _>("share_ratio")?,
                    note: row.try_get::<String, _>("note")?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    const FAKE_SYMBOL: &str = "79979CA";

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
    }

    async fn cleanup() {
        let _ = sqlx::query("DELETE FROM corporate_action WHERE stock_symbol = $1")
            .bind(FAKE_SYMBOL)
            .execute(database::get_connection())
            .await;
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_save_is_idempotent_and_infers_action_type() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_save_is_idempotent_and_infers_action_type：無資料庫連接");
            return;
        }

        let repo = PgCorporateActionRepository::new();
        cleanup().await;

        let mut action = CorporateAction {
            stock_symbol: FAKE_SYMBOL.to_string(),
            effective_date: date(1990, 1, 5),
            share_ratio: dec!(4),
            note: "1:4 分割".to_string(),
        };
        assert_eq!(repo.save(&action).await.expect("save"), 1);

        let saved = repo
            .fetch_by_symbol(FAKE_SYMBOL)
            .await
            .expect("fetch_by_symbol");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].share_ratio, dec!(4));
        assert_eq!(saved[0].note, "1:4 分割");

        // 同一主鍵重送是修正而非新增。
        action.share_ratio = dec!(0.7);
        action.note = "更正為減資三成".to_string();
        repo.save(&action).await.expect("save again");

        let saved = repo
            .fetch_by_symbol(FAKE_SYMBOL)
            .await
            .expect("fetch_by_symbol again");
        assert_eq!(saved.len(), 1, "重複登錄不應新增資料列");
        assert_eq!(saved[0].share_ratio, dec!(0.7));

        // 比例小於 1 應被歸類為減資。
        let action_type: String =
            sqlx::query_scalar("SELECT action_type FROM corporate_action WHERE stock_symbol = $1")
                .bind(FAKE_SYMBOL)
                .fetch_one(database::get_connection())
                .await
                .expect("query action_type");
        assert_eq!(action_type, "capital_reduction");

        cleanup().await;
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_fetch_by_symbol_is_sorted_and_scoped() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_fetch_by_symbol_is_sorted_and_scoped：無資料庫連接");
            return;
        }

        let repo = PgCorporateActionRepository::new();
        cleanup().await;

        for (day, ratio) in [(9_u32, dec!(2)), (3_u32, dec!(4))] {
            repo.save(&CorporateAction {
                stock_symbol: FAKE_SYMBOL.to_string(),
                effective_date: date(1990, 1, day),
                share_ratio: ratio,
                note: String::new(),
            })
            .await
            .expect("save");
        }

        let saved = repo
            .fetch_by_symbol(FAKE_SYMBOL)
            .await
            .expect("fetch_by_symbol");
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[0].effective_date, date(1990, 1, 3));
        assert_eq!(saved[1].effective_date, date(1990, 1, 9));

        // 其他代號查不到這批資料。
        assert!(
            repo.fetch_by_symbol("79979CB")
                .await
                .expect("fetch other symbol")
                .is_empty()
        );

        cleanup().await;
    }
}
