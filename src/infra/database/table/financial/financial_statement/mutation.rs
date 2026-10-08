//! `financial_statement` 的寫入：單筆／批次 upsert、EPS 與 ROE／ROA 更新。

use anyhow::{Result, anyhow};
use sqlx::{QueryBuilder, postgres::PgQueryResult};

use super::FinancialStatement;
use crate::infra::database;

impl FinancialStatement {
    /// 新增或更新單筆財報資料（以 `security_code + year + quarter` 為鍵）。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤。
    pub async fn upsert(self) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO financial_statement (
    security_code, "year", quarter, gross_profit, operating_profit_margin,
    "pre-tax_income", net_income, net_asset_value_per_share, sales_per_share,
    earnings_per_share, profit_before_tax, return_on_equity, return_on_assets,
    created_time, updated_time)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
ON CONFLICT (security_code,"year",quarter) DO UPDATE SET
    gross_profit = EXCLUDED.gross_profit,
    operating_profit_margin = EXCLUDED.operating_profit_margin,
    "pre-tax_income" = EXCLUDED."pre-tax_income",
    net_income = EXCLUDED.net_income,
    net_asset_value_per_share = EXCLUDED.net_asset_value_per_share,
    sales_per_share = EXCLUDED.sales_per_share,
    earnings_per_share = EXCLUDED.earnings_per_share,
    profit_before_tax = EXCLUDED.profit_before_tax,
    return_on_equity = EXCLUDED.return_on_equity,
    return_on_assets = EXCLUDED.return_on_assets,
    updated_time = EXCLUDED.updated_time;
"#;
        sqlx::query(sql)
            .bind(&self.security_code)
            .bind(self.year)
            .bind(&self.quarter)
            .bind(self.gross_profit)
            .bind(self.operating_profit_margin)
            .bind(self.pre_tax_income)
            .bind(self.net_income)
            .bind(self.net_asset_value_per_share)
            .bind(self.sales_per_share)
            .bind(self.earnings_per_share)
            .bind(self.profit_before_tax)
            .bind(self.return_on_equity)
            .bind(self.return_on_assets)
            .bind(self.created_time)
            .bind(self.updated_time)
            .execute(database::get_connection())
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to upsert({:#?}) from database\nsql:{}\n {:?}",
                    self,
                    sql,
                    why
                )
            })
    }

    /// 批次新增或更新多筆財報資料（以 `security_code + year + quarter` 為鍵）。
    ///
    /// 此函式使用 SQLx 的 `QueryBuilder::push_values` 建立多值 `INSERT` 語句，
    /// 並在主鍵衝突時執行對應的欄位更新。相較於單筆逐次寫入，批次寫入能顯著降低資料庫連線延遲。
    ///
    /// # 參數
    /// * `statements` - 要寫入的財報實體清單。
    ///
    /// # 錯誤
    /// 當 SQL 執行失敗或傳入清單為空時回傳錯誤。
    pub async fn batch_upsert(statements: &[Self]) -> Result<PgQueryResult> {
        if statements.is_empty() {
            return Err(anyhow!("Cannot batch_upsert empty statements slice"));
        }

        // 1. 初始化 QueryBuilder 並定義基本 INSERT 欄位
        let mut query_builder = QueryBuilder::new(
            r#"
INSERT INTO financial_statement (
    security_code, "year", quarter, gross_profit, operating_profit_margin,
    "pre-tax_income", net_income, net_asset_value_per_share, sales_per_share,
    earnings_per_share, profit_before_tax, return_on_equity, return_on_assets,
    created_time, updated_time)
"#,
        );

        // 2. 批次推入資料列與對應參數綁定
        query_builder.push_values(statements, |mut b, item| {
            b.push_bind(&item.security_code)
                .push_bind(item.year)
                .push_bind(&item.quarter)
                .push_bind(item.gross_profit)
                .push_bind(item.operating_profit_margin)
                .push_bind(item.pre_tax_income)
                .push_bind(item.net_income)
                .push_bind(item.net_asset_value_per_share)
                .push_bind(item.sales_per_share)
                .push_bind(item.earnings_per_share)
                .push_bind(item.profit_before_tax)
                .push_bind(item.return_on_equity)
                .push_bind(item.return_on_assets)
                .push_bind(item.created_time)
                .push_bind(item.updated_time);
        });

        // 3. 串接衝突更新子句 (Conflict Resolution)
        query_builder.push(
            r#"
ON CONFLICT (security_code,"year",quarter) DO UPDATE SET
    gross_profit = EXCLUDED.gross_profit,
    operating_profit_margin = EXCLUDED.operating_profit_margin,
    "pre-tax_income" = EXCLUDED."pre-tax_income",
    net_income = EXCLUDED.net_income,
    net_asset_value_per_share = EXCLUDED.net_asset_value_per_share,
    sales_per_share = EXCLUDED.sales_per_share,
    earnings_per_share = EXCLUDED.earnings_per_share,
    profit_before_tax = EXCLUDED.profit_before_tax,
    return_on_equity = EXCLUDED.return_on_equity,
    return_on_assets = EXCLUDED.return_on_assets,
    updated_time = EXCLUDED.updated_time;
"#,
        );

        // 4. 建立並執行查詢
        let query = query_builder.build();
        query
            .execute(database::get_connection())
            .await
            .map_err(|why| anyhow!("Failed to batch_upsert statements from database: {:?}", why))
    }

    /// 僅新增或更新每股盈餘欄位。
    ///
    /// 適合只抓到 EPS 的資料來源，避免覆寫其他欄位。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤。
    pub async fn upsert_earnings_per_share(&self) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO financial_statement (
    security_code, "year", quarter, earnings_per_share, created_time, updated_time)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (security_code,"year",quarter)
DO UPDATE SET 
    earnings_per_share = excluded.earnings_per_share,
    updated_time = EXCLUDED.updated_time;
"#;
        sqlx::query(sql)
            .bind(&self.security_code)
            .bind(self.year)
            .bind(&self.quarter)
            .bind(self.earnings_per_share)
            .bind(self.created_time)
            .bind(self.updated_time)
            .execute(database::get_connection())
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to upsert_earnings_per_share({:#?}) from database\nsql:{}\n {:?}",
                    self,
                    sql,
                    why
                )
            })
    }

    /// 補寫年度匯總 EPS（`quarter = ''`）。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤。
    pub async fn upsert_annual_eps(&self) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO financial_statement (
    security_code, "year", quarter, earnings_per_share, profit_before_tax, sales_per_share, created_time, updated_time)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
ON CONFLICT (security_code,"year",quarter) DO NOTHING;
"#;
        sqlx::query(sql)
            .bind(&self.security_code)
            .bind(self.year)
            .bind(&self.quarter)
            .bind(self.earnings_per_share)
            .bind(self.profit_before_tax)
            .bind(self.sales_per_share)
            .bind(self.created_time)
            .bind(self.updated_time)
            .execute(database::get_connection())
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to upsert_annual_eps({:#?}) from database\nsql:{}\n {:?}",
                    self,
                    sql,
                    why
                )
            })
    }

    /// 更新既有財報列的 ROE 與 ROA。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤。
    pub async fn update_roe_roa(&self) -> Result<PgQueryResult> {
        let sql = r#"
UPDATE
    financial_statement
SET
    return_on_equity = $4, return_on_assets = $5, updated_time = $6
WHERE
    security_code = $1 AND "year" = $2 AND quarter = $3
"#;
        sqlx::query(sql)
            .bind(&self.security_code)
            .bind(self.year)
            .bind(&self.quarter)
            .bind(self.return_on_equity)
            .bind(self.return_on_assets)
            .bind(self.updated_time)
            .execute(database::get_connection())
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to update_roe_roa({:#?}) from database\nsql:{}\n {:?}",
                    self,
                    sql,
                    why
                )
            })
    }
}
