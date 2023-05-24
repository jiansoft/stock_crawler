use crate::internal::database::DB;
use anyhow::Result;
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use sqlx::{postgres::PgRow, Postgres, Row, Transaction};

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 持股股息發放記錄表 原表名 dividend_record_detail
pub struct DividendRecordDetail {
    pub serial: i64,
    /// 庫存編號
    pub stock_ownership_details_serial: i64,
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

impl DividendRecordDetail {
    pub fn new(
        stock_ownership_details_serial: i64,
        year: i32,
        cash: Decimal,
        stock: Decimal,
        stock_money: Decimal,
        total: Decimal,
    ) -> Self {
        DividendRecordDetail {
            serial: 0,
            stock_ownership_details_serial,
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
    pub async fn upsert(&mut self, tx: Option<Transaction<'_, Postgres>>) -> Result<i64> {
        let sql = r#"
        insert into dividend_record_detail (stock_ownership_details_serial, "year", cash, stock_money, stock, total)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (stock_ownership_details_serial, "year") DO UPDATE SET
        total = EXCLUDED.total,
        cash = EXCLUDED.cash,
        stock_money = EXCLUDED.stock_money,
        stock = EXCLUDED.stock,
        updated_time = now()
        RETURNING serial;
    "#;
        let query = sqlx::query_as(sql)
            .bind(self.stock_ownership_details_serial)
            .bind(self.year)
            .bind(self.cash)
            .bind(self.stock_money)
            .bind(self.stock)
            .bind(self.total);
        let row: (i64,) = match tx {
            None => query.fetch_one(&DB.pool).await?,
            Some(mut t) => query.fetch_one(&mut t).await?,
        };

        self.serial = row.0;

        Ok(self.serial)
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
where stock_ownership_details_serial = $1;
        "#,
        )
        .bind(self.stock_ownership_details_serial)
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

impl Default for DividendRecordDetail {
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

impl Clone for DividendRecordDetail {
    fn clone(&self) -> Self {
        DividendRecordDetail {
            serial: self.serial,
            stock_ownership_details_serial: self.stock_ownership_details_serial,
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
