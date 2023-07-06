pub(crate) mod extension;

use crate::internal::database::{self, table::dividend_record_detail::extension::CumulateDividend};
use anyhow::Result;
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};

#[derive(sqlx::Type, sqlx::FromRow, Debug, Copy)]
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
    pub created_time: DateTime<Local>,
    pub updated_time: DateTime<Local>,
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
        //dbg!(row);
        self.serial = row.0;

        //dbg!(*self);

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
            created_time: self.created_time,
            updated_time: self.updated_time,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::logging;

    #[tokio::test]
    async fn test_calculate_cumulate_dividend() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 calculate_cumulate_dividend".to_string());
        let drd = DividendRecordDetail::new(
            27,
            2022,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
        );
        let mut tx_option: Option<Transaction<Postgres>> =
            Some(database::get_connection().begin().await.unwrap());
        match drd.fetch_cumulate_dividend(&mut tx_option).await {
            Ok(cd) => {
                logging::debug_file_async(format!("cd: {:?}", cd));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        if let Some(tx) = tx_option {
            tx.commit().await.unwrap();
        }

        logging::debug_file_async("結束 calculate_cumulate_dividend".to_string());
    }
}
