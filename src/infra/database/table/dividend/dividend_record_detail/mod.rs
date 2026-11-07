use anyhow::Result;
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};

use crate::infra::database::{self, table::dividend_record_detail::extension::CumulateDividend};

pub(crate) mod extension;

#[derive(sqlx::Type, sqlx::FromRow, Debug, Clone, Copy)]
/// 持股股息發放記錄表 原表名 dividend_record_detail
pub struct DividendRecordDetail {
    /// 主鍵序號。
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
    /// 建立時間。
    pub created_time: DateTime<Local>,
    /// 最後更新時間。
    pub updated_time: DateTime<Local>,
}

impl DividendRecordDetail {
    /// 建立一筆持股年度股利總計資料。
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
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }

    /// 更新持股股息發放記錄
    pub async fn upsert(&mut self, tx: &mut Option<Transaction<'_, Postgres>>) -> Result<i64> {
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
            None => query.fetch_one(database::get_connection()).await?,
            Some(t) => query.fetch_one(&mut **t).await?,
        };
        self.serial = row.0;

        Ok(self.serial)
    }

    /// 計算指定股票其累積的領取股利
    pub async fn fetch_cumulate_dividend(
        &self,
        tx: &mut Option<Transaction<'_, Postgres>>,
    ) -> Result<CumulateDividend> {
        CumulateDividend::fetch_cumulate_dividend(self.stock_ownership_details_serial, tx).await
    }
}

#[cfg(test)]
mod tests;
