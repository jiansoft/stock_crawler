use crate::internal::database::model::dividend_record_detail;
use crate::internal::database::DB;
use anyhow::Result;
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use sqlx::postgres::PgRow;
use sqlx::Row;

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 股票庫存 原表名 stock_ownership_details
pub struct Entity {
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
    pub cumulate_cash: Decimal,
    /// 累積股票股利(股)
    pub cumulate_stock: Decimal,
    /// 累積股票股利(元)
    pub cumulate_stock_money: Decimal,
    /// 總計累積股利(元)
    pub cumulate_total: Decimal,
    pub created_time: DateTime<Local>,
}

impl Entity {
    pub fn new() -> Self {
        Entity {
            serial: 0,
            security_code: Default::default(),
            member_id: 0,
            share_quantity: Default::default(),
            share_price_average: Default::default(),
            holding_cost: Default::default(),
            is_sold: false,
            cumulate_cash: Default::default(),
            cumulate_stock: Default::default(),
            cumulate_stock_money: Default::default(),
            cumulate_total: Default::default(),
            created_time: Default::default(),
        }
    }

    /// 更新指定股票累積的股利
    pub async fn update_cumulate_dividends(&self) -> Result<()> {
        let sql = r#"
update
    stock_ownership_details
set
    cumulate_dividends_cash = $2,
    cumulate_dividends_stock= $3,
    cumulate_dividends_stock_money= $4,
    cumulate_dividends_total= $5
where
    "Id" = $1;
"#;

        sqlx::query(sql)
            .bind(self.serial)
            .bind(self.cumulate_cash)
            .bind(self.cumulate_stock)
            .bind(self.cumulate_stock_money)
            .bind(self.cumulate_total)
            .execute(&DB.pool)
            .await?;
        Ok(())
    }

    /// 計算指定年份與股票其領取的股利
    pub async fn calculate_dividend(&self, year: i32) -> Result<dividend_record_detail::Entity> {
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
    and ("ex-dividend_date1" >= $3 or "ex-dividend_date2" >= $4)
    and ("ex-dividend_date1" <= $5 );
        "#,
        )
        .bind(&self.security_code)
        .bind(year)
        .bind(self.created_time.format("%Y-%m-%d 00:00:00").to_string())
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
        股票股利＝1張x1000股x股利0.36=360股
        (股票股利須除以發行面額10元)
        20048 *(0.5/10)
        */

        let number_of_shares_held = Decimal::new(self.share_quantity, 0);
        let dividend_cash = dividend.0 * number_of_shares_held;
        let dividend_stock = dividend.1 * number_of_shares_held / Decimal::new(10, 0);
        let dividend_stock_money = dividend.1 * number_of_shares_held;
        let dividend_total = dividend.2 * number_of_shares_held;
        let drd = dividend_record_detail::Entity::new(
            self.serial,
            year,
            dividend_cash,
            dividend_stock,
            dividend_stock_money,
            dividend_total,
        );

        if drd.total != Decimal::ZERO {
            drd.upsert().await?;
        }

        Ok(drd)
    }
}

impl Default for Entity {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for Entity {
    fn clone(&self) -> Self {
        Entity {
            serial: self.serial,
            security_code: self.security_code.to_string(),
            member_id: self.member_id,
            share_quantity: self.share_quantity,
            share_price_average: self.share_price_average,
            holding_cost: self.holding_cost,
            is_sold: self.is_sold,
            cumulate_cash: self.cumulate_cash,
            cumulate_stock: self.cumulate_stock,
            cumulate_stock_money: self.cumulate_stock_money,
            cumulate_total: self.cumulate_total,
            created_time: self.created_time,
        }
    }
}

/// 取得庫存股票的數據
pub async fn fetch() -> Result<Vec<Entity>> {
    let answers = sqlx::query(
        r#"
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
where is_sold = false
        "#,
    )
    .try_map(|row: PgRow| {
        Ok(Entity {
            serial: row.try_get("serial")?,
            security_code: row.try_get("security_code")?,
            member_id: row.try_get("member_id")?,
            share_quantity: row.try_get("share_quantity")?,
            share_price_average: row.try_get("share_price_average")?,
            holding_cost: row.try_get("holding_cost")?,
            created_time: row.try_get("created_time")?,
            is_sold: false,
            cumulate_cash: row.try_get("cumulate_dividends_cash")?,
            cumulate_stock: row.try_get("cumulate_dividends_stock")?,
            cumulate_stock_money: row.try_get("cumulate_dividends_stock_money")?,
            cumulate_total: row.try_get("cumulate_dividends_total")?,
        })
    })
    .fetch_all(&DB.pool)
    .await?;

    Ok(answers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;

    #[tokio::test]
    async fn test_fetch_stock_inventory() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 fetch_stock_inventory".to_string());
        let r = fetch().await;
        if let Ok(result) = r {
            for e in result {
                logging::info_file_async(format!("{:#?} ", e));
            }
        } else if let Err(err) = r {
            logging::error_file_async(format!("{:#?} ", err));
        }
        //logging::info_file_async("結束 fetch".to_string());
    }
}
