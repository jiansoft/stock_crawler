use crate::internal::database::DB;
use anyhow::Result;
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use sqlx::{postgres::PgRow, Row};

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 持股股息發放記錄表 原表名 dividend_record_detail
pub struct Entity {
    /// 庫存編號
    pub favorite_id: i64,
    /// 領取年度
    pub year: i32,
    /// 現金股利(元)
    pub cash: Decimal,
    /// 股票股利(股)
    pub stock: Decimal,
    /// 股票股利(元)
    pub stock_money: Decimal,
    /// 合計股利(元)
    pub total: Decimal,
    pub create_time: DateTime<Local>,
    pub update_time: DateTime<Local>,
}

impl Entity {
    pub fn new(
        favorite_id: i64,
        year: i32,
        cash: Decimal,
        stock: Decimal,
        stock_money: Decimal,
        total: Decimal,
    ) -> Self {
        Entity {
            favorite_id,
            year,
            cash,
            stock,
            stock_money,
            total,
            create_time: Local::now(),
            update_time: Local::now(),
        }
    }

    /// 更新持股股息發放記錄
    pub async fn upsert(&self) -> Result<()> {
        let sql = r#"
        insert into dividend_record_detail (favorite_id, "year", cash, stock_money, stock, total)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (favorite_id, "year") DO UPDATE SET
        total = EXCLUDED.total,
        cash = EXCLUDED.cash,
        stock_money = EXCLUDED.stock_money,
        stock = EXCLUDED.stock,
        updated_time = now();
    "#;
        sqlx::query(sql)
            .bind(self.favorite_id)
            .bind(self.year)
            .bind(self.cash)
            .bind(self.stock_money)
            .bind(self.stock)
            .bind(self.total)
            .execute(&DB.pool)
            .await?;

        Ok(())
    }

    /// 計算指定股票其累積的領取股利
    pub async fn calculate_cumulate_dividend(
        &self,
    ) -> Result<(Decimal, Decimal, Decimal, Decimal)> {
        let dividend = sqlx::query(
            r#"
select COALESCE(sum(cash), 0)        as cash,
       COALESCE(sum(stock_money), 0) as stock_money,
       COALESCE(sum(stock), 0)       as stock,
       COALESCE(sum(total), 0)       as total
from dividend_record_detail
where favorite_id = $1;
        "#,
        )
        .bind(self.favorite_id)
        .try_map(|row: PgRow| {
            let cash: Decimal = row.try_get("cash")?;
            let stock_money: Decimal = row.try_get("stock_money")?;
            let stock: Decimal = row.try_get("stock")?;
            let total: Decimal = row.try_get("total")?;
            Ok((cash, stock_money, stock, total))
        })
        .fetch_one(&DB.pool)
        .await?;

        Ok(dividend)
    }
}

impl Default for Entity {
    fn default() -> Self {
        Self::new(
            0,
            0,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
        )
    }
}

impl Clone for Entity {
    fn clone(&self) -> Self {
        Entity {
            favorite_id: self.favorite_id,
            year: self.year,
            cash: self.cash,
            stock: self.stock,
            stock_money: self.stock_money,
            total: self.total,
            create_time: self.create_time,
            update_time: self.update_time,
        }
    }
}
