use crate::internal::database::DB;
use anyhow::Result;
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use sqlx::{postgres::PgQueryResult, Postgres, Transaction};

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 股票庫存 原表名 stock_ownership_details
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
        let rows = query.fetch_all(&DB.pool).await?;

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
        tx: Option<Transaction<'_, Postgres>>,
    ) -> Result<PgQueryResult> {
        let sql = r#"
update
    stock_ownership_details
set
    cumulate_dividends_cash = $2,
    cumulate_dividends_stock= $3,
    cumulate_dividends_stock_money= $4,
    cumulate_dividends_total= $5
where
    serial = $1;
"#;
        let query = sqlx::query(sql)
            .bind(self.serial)
            .bind(self.cumulate_dividends_cash)
            .bind(self.cumulate_dividends_stock)
            .bind(self.cumulate_dividends_stock_money)
            .bind(self.cumulate_dividends_total);
        let result = match tx {
            None => query.execute(&DB.pool).await?,
            Some(mut t) => query.execute(&mut t).await?,
        };

        Ok(result)
    }

    /*/// 計算指定年份與股票其領取的股利，如果股利並非零時將數據更新到 dividend_record_detail 表
        pub async fn calculate_dividend_and_upsert(
            &self,
            year: i32,
        ) -> Result<dividend_record_detail::DividendRecordDetail> {
            //計算股票於該年度可以領取的股利
            let dividend = sqlx::query(
                r#"
    select
        COALESCE(sum(cash_dividend),0) as cash,
        COALESCE(sum(stock_dividend),0) as stock,
        COALESCE(sum(sum),0) as sum
    from dividend
    where security_code = $1
        and year = $2
        and ("ex-dividend_date1" >= $3 or "ex-dividend_date2" >= $3)
        and ("ex-dividend_date1" <= $4);
            "#,
            )
            .bind(&self.security_code)
            .bind(year)
            .bind(self.created_time.format("%Y-%m-%d 00:00:00").to_string())
            .bind(Local::now().format("%Y-%m-%d 00:00:00").to_string())
            .try_map(|row: PgRow| {
                let cash: Decimal = row.try_get("cash")?;
                let stock: Decimal = row.try_get("stock")?;
                let sum: Decimal = row.try_get("sum")?;
                Ok((cash, stock, sum))
            })
            .fetch_one(&DB.pool)
            .await?;

            /*
            某公司股價100元配現金0.7元、配股3.6元(以一張為例)
            現金股利＝1張ｘ1000股x股利0.7元=700元
            股票股利＝1張x1000股x股利0.36=360股 (股票股利須除以發行面額10元)
            20048 *(0.5/10)
            */

            let number_of_shares_held = Decimal::new(self.share_quantity, 0);
            let dividend_cash = dividend.0 * number_of_shares_held;
            let dividend_stock = dividend.1 * number_of_shares_held / Decimal::new(10, 0);
            let dividend_stock_money = dividend.1 * number_of_shares_held;
            let dividend_total = dividend.2 * number_of_shares_held;
            let mut drd = dividend_record_detail::DividendRecordDetail::new(
                self.serial,
                year,
                dividend_cash,
                dividend_stock,
                dividend_stock_money,
                dividend_total,
            );

            let mut tx_option: Option<Transaction<Postgres>> = Some(DB.pool.begin().await?);

            let dividend_record_detail_serial = match drd.upsert(tx_option.take()).await {
                Ok(serial) => serial,
                Err(why) => {
                    if let Some(tx) = tx_option {
                        tx.rollback().await?;
                    }
                    return Err(anyhow!(
                        "Failed to execute upsert query for dividend_record_detail because {:?}",
                        why
                    ));
                }
            };

            let mut e = model::dividend::Dividend::new();
            e.security_code = self.security_code.to_string();
            e.year = year;
            let dividends = model::dividend::Dividend::fetch_dividends_summary_by_date(
                &self.security_code,
                e.year,
                self.created_time,
            )
            .await?;
            for dividend in dividends {
                //寫入領取細節
                let dividend_cash = dividend.cash_dividend * number_of_shares_held;
                let dividend_stock =
                    dividend.stock_dividend * number_of_shares_held / Decimal::new(10, 0);
                let dividend_stock_money = dividend.stock_dividend * number_of_shares_held;
                let dividend_total = dividend.sum * number_of_shares_held;

                let mut e = dividend_record_detail_more::DividendRecordDetailMore::new(
                    dividend_record_detail_serial,
                    dividend.serial,
                    dividend_cash,
                    dividend_stock,
                    dividend_stock_money,
                    dividend_total,
                );
                e.upsert(tx_option.take()).await?;
            }

            if let Some(tx) = tx_option {
                tx.commit().await?;
            }

            Ok(drd)
        }*/
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

    use super::*;
    use crate::internal::logging;

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
