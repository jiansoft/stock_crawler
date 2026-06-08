use crate::domain::portfolio::entity::{
    ReceivedDividend, ReceivedDividendItem, StockOwnershipDetail,
};
use crate::domain::portfolio::repository::PortfolioRepository;
use crate::infra::database;
use anyhow::{Context, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use sqlx::{postgres::PgRow, QueryBuilder, Row};

/// 基於 PostgreSQL 的持股倉儲實現 (PgPortfolioRepository)。
pub struct PgPortfolioRepository;

impl PgPortfolioRepository {
    /// 建立新的 PgPortfolioRepository 實例。
    pub fn new() -> Self {
        PgPortfolioRepository
    }

    /// 將資料庫的 `PgRow` 轉換成領域實體 `StockOwnershipDetail`。
    fn row_to_entity(row: PgRow) -> Result<StockOwnershipDetail, sqlx::Error> {
        Ok(StockOwnershipDetail::reconstitute(
            row.try_get("serial")?,
            row.try_get("security_code")?,
            row.try_get("member_id")?,
            row.try_get("share_quantity")?,
            row.try_get("share_price_average")?,
            row.try_get("current_cost_per_share")?,
            row.try_get("holding_cost")?,
            row.try_get("is_sold")?,
            row.try_get("cumulate_dividends_cash")?,
            row.try_get("cumulate_dividends_stock")?,
            row.try_get("cumulate_dividends_stock_money")?,
            row.try_get("cumulate_dividends_total")?,
            row.try_get("created_time")?,
        ))
    }
}

impl Default for PgPortfolioRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PortfolioRepository for PgPortfolioRepository {
    /// 依證券代號清單查詢所有「未售出」的持股明細。
    async fn fetch_active_holdings(
        &self,
        security_codes: Option<Vec<String>>,
    ) -> Result<Vec<StockOwnershipDetail>> {
        let mut query_builder = QueryBuilder::new(
            r#"
            SELECT
                serial, member_id, security_code, share_quantity, holding_cost, created_time,
                share_price_average, current_cost_per_share, is_sold,
                cumulate_dividends_cash, cumulate_dividends_stock, cumulate_dividends_stock_money, cumulate_dividends_total
            FROM stock_ownership_details
            WHERE is_sold = false
            "#,
        );

        if let Some(scs) = security_codes {
            if !scs.is_empty() {
                query_builder.push(" AND security_code = ANY(");
                query_builder.push_bind(scs);
                query_builder.push(")");
            }
        }

        let query = query_builder.build();
        let rows = query
            .try_map(Self::row_to_entity)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch active holdings from PostgreSQL")?;

        Ok(rows)
    }

    /// 依序號尋找單筆持股明細。
    async fn find_holding_by_serial(&self, serial: i64) -> Result<Option<StockOwnershipDetail>> {
        let sql = r#"
            SELECT
                serial, member_id, security_code, share_quantity, holding_cost, created_time,
                share_price_average, current_cost_per_share, is_sold,
                cumulate_dividends_cash, cumulate_dividends_stock, cumulate_dividends_stock_money, cumulate_dividends_total
            FROM stock_ownership_details
            WHERE serial = $1
        "#;
        let row_opt = sqlx::query(sql)
            .bind(serial)
            .try_map(Self::row_to_entity)
            .fetch_optional(database::get_connection())
            .await
            .context("Failed to find holding by serial")?;
        Ok(row_opt)
    }

    /// 更新持股明細之累積已領股利數值。
    async fn update_holding_dividends(&self, holding: &StockOwnershipDetail) -> Result<()> {
        let sql = r#"
            UPDATE stock_ownership_details
            SET
                cumulate_dividends_cash = $2,
                cumulate_dividends_stock = $3,
                cumulate_dividends_stock_money = $4,
                cumulate_dividends_total = $5
            WHERE
                serial = $1;
        "#;
        sqlx::query(sql)
            .bind(holding.serial)
            .bind(holding.cumulate_dividends_cash)
            .bind(holding.cumulate_dividends_stock)
            .bind(holding.cumulate_dividends_stock_money)
            .bind(holding.cumulate_dividends_total)
            .execute(database::get_connection())
            .await
            .context("Failed to update holding dividends in PostgreSQL")?;
        Ok(())
    }

    /// 儲存持股年度已領股利總計及其各配發宣告項目。
    async fn save_received_dividend(
        &self,
        summary: &ReceivedDividend,
        items: &[ReceivedDividendItem],
    ) -> Result<()> {
        let mut tx = database::get_tx().await?;

        // 1. 寫入或更新年度總計 (dividend_record_detail)
        let sql_summary = r#"
            INSERT INTO dividend_record_detail (
                stock_ownership_details_serial, "year", cash, stock_money, stock, total, created_time, updated_time)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (stock_ownership_details_serial, "year") DO UPDATE SET
                total = EXCLUDED.total,
                cash = EXCLUDED.cash,
                stock_money = EXCLUDED.stock_money,
                stock = EXCLUDED.stock,
                updated_time = EXCLUDED.updated_time
            RETURNING serial;
        "#;

        let row: (i64,) = sqlx::query_as(sql_summary)
            .bind(summary.stock_ownership_details_serial)
            .bind(summary.year)
            .bind(summary.cash)
            .bind(summary.stock_money)
            .bind(summary.stock)
            .bind(summary.total)
            .bind(summary.created_time)
            .bind(summary.updated_time)
            .fetch_one(&mut *tx)
            .await
            .context("Failed to save received dividend summary")?;

        let record_detail_serial = row.0;

        // 2. 寫入或更新配發細項 (dividend_record_detail_more)
        let sql_item = r#"
            INSERT INTO dividend_record_detail_more (
                stock_ownership_details_serial, dividend_record_detail_serial, dividend_serial,
                cash, stock_money, stock, total, created_time, updated_time)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (stock_ownership_details_serial, dividend_record_detail_serial, dividend_serial) DO UPDATE SET
                total = EXCLUDED.total,
                cash = EXCLUDED.cash,
                stock_money = EXCLUDED.stock_money,
                stock = EXCLUDED.stock,
                updated_time = EXCLUDED.updated_time;
        "#;

        for item in items {
            sqlx::query(sql_item)
                .bind(item.stock_ownership_details_serial)
                .bind(record_detail_serial) // 使用剛產生的總計表序號
                .bind(item.dividend_serial)
                .bind(item.cash)
                .bind(item.stock_money)
                .bind(item.stock)
                .bind(item.total)
                .bind(item.created_time)
                .bind(item.updated_time)
                .execute(&mut *tx)
                .await
                .context("Failed to save received dividend item")?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// 刪除指定持股在特定年度的已領股利總計與細項明細。
    async fn delete_received_dividend(&self, holding_serial: i64, year: i32) -> Result<()> {
        let mut tx = database::get_tx().await?;

        // 1. 先刪除細項明細 (dividend_record_detail_more)
        let sql_delete_more = r#"
            DELETE FROM dividend_record_detail_more
            WHERE stock_ownership_details_serial = $1
              AND dividend_record_detail_serial IN (
                  SELECT serial
                  FROM dividend_record_detail
                  WHERE stock_ownership_details_serial = $1 AND year = $2
              );
        "#;
        sqlx::query(sql_delete_more)
            .bind(holding_serial)
            .bind(year)
            .execute(&mut *tx)
            .await
            .context("Failed to delete received dividend items")?;

        // 2. 再刪除年度總計 (dividend_record_detail)
        let sql_delete_summary = r#"
            DELETE FROM dividend_record_detail
            WHERE stock_ownership_details_serial = $1 AND year = $2;
        "#;
        sqlx::query(sql_delete_summary)
            .bind(holding_serial)
            .bind(year)
            .execute(&mut *tx)
            .await
            .context("Failed to delete received dividend summary")?;

        tx.commit().await?;
        Ok(())
    }

    /// 計算指定持股之累積已領取股利總額。
    async fn calculate_accumulated_dividends(
        &self,
        holding_serial: i64,
    ) -> Result<(Decimal, Decimal, Decimal, Decimal)> {
        let sql = r#"
            SELECT
                COALESCE(sum(cash), 0)        as cash,
                COALESCE(sum(stock), 0)       as stock,
                COALESCE(sum(stock_money), 0) as stock_money,
                COALESCE(sum(total), 0)       as total
            FROM dividend_record_detail
            WHERE stock_ownership_details_serial = $1;
        "#;
        let row = sqlx::query(sql)
            .bind(holding_serial)
            .map(|r: PgRow| {
                (
                    r.get::<Decimal, _>("cash"),
                    r.get::<Decimal, _>("stock"),
                    r.get::<Decimal, _>("stock_money"),
                    r.get::<Decimal, _>("total"),
                )
            })
            .fetch_one(database::get_connection())
            .await
            .context("Failed to calculate accumulated dividends")?;
        Ok(row)
    }
}
