use anyhow::Result;
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use sqlx::{Postgres, postgres::PgQueryResult, Transaction};

use crate::internal::database;

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 股票庫存(持股名細) 原表名 stock_ownership_details
pub struct StockOwnershipDetail {
    /// 序號 原 Id
    pub serial: i64,
    /// 股票代號
    pub security_code: String,
    /// 當會員編號
    pub member_id: i64,
    /// 持有股數
    pub share_quantity: i64,
    /// 每股成本
    pub share_price_average: Decimal,
    /// 買入成本
    pub holding_cost: Decimal,
    /// 是否賣出
    pub is_sold: bool,
    /// 累積現金股利(元)
    pub cumulate_dividends_cash: Decimal,
    /// 累積股票股利(股)
    pub cumulate_dividends_stock: Decimal,
    /// 累積股票股利(元)
    pub cumulate_dividends_stock_money: Decimal,
    /// 總計累積股利(元)
    pub cumulate_dividends_total: Decimal,
    pub created_time: DateTime<Local>,
}

impl StockOwnershipDetail {
    pub fn new() -> Self {
        StockOwnershipDetail {
            serial: 0,
            security_code: Default::default(),
            member_id: 0,
            share_quantity: Default::default(),
            share_price_average: Default::default(),
            holding_cost: Default::default(),
            is_sold: false,
            cumulate_dividends_cash: Default::default(),
            cumulate_dividends_stock: Default::default(),
            cumulate_dividends_stock_money: Default::default(),
            cumulate_dividends_total: Default::default(),
            created_time: Default::default(),
        }
    }

    /// 取得庫存股票的數據
    pub async fn fetch(security_codes: Option<Vec<String>>) -> Result<Vec<StockOwnershipDetail>> {
        let base_sql = "
SELECT
    serial,
    member_id,
    security_code,
    share_quantity,
    holding_cost,
    created_time,
    share_price_average,
    is_sold,
    cumulate_dividends_cash,
    cumulate_dividends_stock,
    cumulate_dividends_stock_money,
    cumulate_dividends_total
FROM stock_ownership_details
WHERE is_sold = false";
        let (sql, bind_params) = security_codes
            .map(|scs| {
                let params = scs
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("${}", i + 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    format!("{} AND security_code IN ({})", base_sql, params),
                    scs,
                )
            })
            .unwrap_or((base_sql.to_string(), vec![]));

        let query = sqlx::query_as::<_, StockOwnershipDetail>(&sql);
        let query = bind_params
            .into_iter()
            .fold(query, |q, param| q.bind(param));
        let rows = query.fetch_all(database::get_connection()).await?;

        Ok(rows)

        /*
             let mut sql = "
        select serial,
           member_id,
           security_code,
           share_quantity,
           holding_cost,
           created_time,
           share_price_average,
           is_sold,
           cumulate_dividends_cash,
           cumulate_dividends_stock,
           cumulate_dividends_stock_money,
           cumulate_dividends_total
        from stock_ownership_details
        where is_sold = false "
                .to_string();
            if let Some(scs) = security_codes {
                let params = scs
                    .iter()
                    .enumerate()
                    .map(|(i, _id)| format!("${}", i + 1))
                    .collect::<Vec<_>>()
                    .join(", ");

                sql = format!("{} AND security_code IN ({})", sql, params)
                    .as_str()
                    .parse()?;

                let mut query = sqlx::query_as::<_, Entity>(&sql);
                for i in scs {
                    query = query.bind(i);
                }

                let rows = query.fetch_all(&DB.pool).await?;
                return Ok(rows);
            }

            let rows = sqlx::query_as::<_, Entity>(&sql)
                .fetch_all(&DB.pool)
                .await?;

            Ok(rows)
        */
    }

    /// 更新指定股票累積的股利
    pub async fn update_cumulate_dividends(
        &self,
        tx: &mut Option<Transaction<'_, Postgres>>,
    ) -> Result<PgQueryResult> {
        let sql = r#"
UPDATE stock_ownership_details
SET
    cumulate_dividends_cash = $2,
    cumulate_dividends_stock = $3,
    cumulate_dividends_stock_money = $4,
    cumulate_dividends_total = $5
WHERE
    serial = $1
"#;
        let query = sqlx::query(sql)
            .bind(self.serial)
            .bind(self.cumulate_dividends_cash)
            .bind(self.cumulate_dividends_stock)
            .bind(self.cumulate_dividends_stock_money)
            .bind(self.cumulate_dividends_total);
        let result = match tx {
            None => query.execute(database::get_connection()).await?,
            Some(t) => query.execute(&mut **t).await?,
        };

        Ok(result)
    }
}

impl Default for StockOwnershipDetail {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for StockOwnershipDetail {
    fn clone(&self) -> Self {
        StockOwnershipDetail {
            serial: self.serial,
            security_code: self.security_code.to_string(),
            member_id: self.member_id,
            share_quantity: self.share_quantity,
            share_price_average: self.share_price_average,
            holding_cost: self.holding_cost,
            is_sold: self.is_sold,
            cumulate_dividends_cash: self.cumulate_dividends_cash,
            cumulate_dividends_stock: self.cumulate_dividends_stock,
            cumulate_dividends_stock_money: self.cumulate_dividends_stock_money,
            cumulate_dividends_total: self.cumulate_dividends_total,
            created_time: self.created_time,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::internal::logging;

    use super::*;

    #[tokio::test]
    async fn test_fetch_stock_inventory() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch".to_string());
        let r = StockOwnershipDetail::fetch(Some(vec!["2330".to_string()])).await;
        if let Ok(result) = r {
            for e in result {
                logging::debug_file_async(format!("{:?} ", e));
            }
        } else if let Err(err) = r {
            logging::debug_file_async(format!("{:#?} ", err));
        }
        logging::debug_file_async("結束 fetch".to_string());
    }
}
