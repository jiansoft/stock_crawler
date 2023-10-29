use anyhow::Result;
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};

use crate::internal::database;

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 持股股息發放記錄表 原表名 dividend_record_detail_more
pub struct DividendRecordDetailMore {
    pub serial: i64,
    /// 持股名細表的編號
    pub stock_ownership_details_serial: i64,
    /// 持股股息發放記錄表的編號(總計表)
    pub dividend_record_detail_serial: i64,
    /// 股利發放明細表的編號-計算現金股利、股票股利、股票股利、合計股利的參考數據源
    pub dividend_serial: i64,
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

impl DividendRecordDetailMore {
    pub fn new(
        stock_ownership_details_serial: i64,
        dividend_record_detail_serial: i64,
        dividend_serial: i64,
        cash: Decimal,
        stock: Decimal,
        stock_money: Decimal,
        total: Decimal,
    ) -> Self {
        DividendRecordDetailMore {
            serial: 0,
            stock_ownership_details_serial,
            dividend_record_detail_serial,
            dividend_serial,
            cash,
            stock,
            stock_money,
            total,
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }

    /// 更新持股股息發放明細記錄
    pub async fn upsert(&mut self, tx: &mut Option<Transaction<'_, Postgres>>) -> Result<i64> {
        let sql = r#"
INSERT INTO dividend_record_detail_more (
    stock_ownership_details_serial,
    dividend_record_detail_serial,
    dividend_serial, cash, stock_money,
    stock, total, created_time, updated_time)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
ON CONFLICT (stock_ownership_details_serial, dividend_record_detail_serial, dividend_serial)
DO UPDATE SET
    total = EXCLUDED.total,
    cash = EXCLUDED.cash,
    stock_money = EXCLUDED.stock_money,
    stock = EXCLUDED.stock,
    updated_time = EXCLUDED.updated_time
RETURNING serial;
"#;
        let query = sqlx::query_as(sql)
            .bind(self.stock_ownership_details_serial)
            .bind(self.dividend_record_detail_serial)
            .bind(self.dividend_serial)
            .bind(self.cash)
            .bind(self.stock_money)
            .bind(self.stock)
            .bind(self.total)
            .bind(self.created_time)
            .bind(self.updated_time);

        let row: (i64,) = match tx {
            None => query.fetch_one(database::get_connection()).await?,
            Some(t) => query.fetch_one(&mut **t).await?,
        };

        self.serial = row.0;

        Ok(self.serial)
    }
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_upsert() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 test_upsert".to_string());

        let mut e = DividendRecordDetailMore::new(
            1,
            2,
            3,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
        );
        let mut tx_option: Option<Transaction<Postgres>> = database::get_tx().await.ok();
        match e.upsert(&mut tx_option).await {
            Ok(word_id) => {
                logging::debug_file_async(format!("serial:{} e:{:#?}", word_id, &e));

                if let Some(mut tx) = tx_option.take() {
                    let _ =
                        sqlx::query("delete from dividend_record_detail_more where serial = $1;")
                            .bind(word_id)
                            .execute(&mut *tx)
                            .await;
                }
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to upsert because:{:?}", why));
                if let Some(tx) = tx_option.take() {
                    tx.rollback().await.unwrap();
                }
            }
        }

        if let Some(tx) = tx_option {
            tx.commit().await.unwrap();
        }

        logging::debug_file_async("結束 test_upsert".to_string());
    }
}
