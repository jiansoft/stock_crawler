use crate::internal::database::DB;
use anyhow::Result;
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use sqlx::postgres::PgRow;
use sqlx::Row;

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 股票庫存 原表名 Favorite
pub struct Entity {
    /// 序號 原 Id
    pub serial: i64,
    /// 股票代號
    pub security_code: String,
    /// 當會員編號
    pub member_id: i64,
    /// 持有股數
    pub number_of_shares_held: i64,
    /// 每股成本
    pub cost_per_share: Decimal,
    /// 買入成本
    pub cost: Decimal,
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
    pub create_time: DateTime<Local>,
}

impl Entity {
    pub fn new() -> Self {
        Entity {
            serial: 0,
            security_code: Default::default(),
            member_id: 0,
            number_of_shares_held: Default::default(),
            cost_per_share: Default::default(),
            cost: Default::default(),
            is_sold: false,
            cumulate_dividends_cash: Default::default(),
            cumulate_dividends_stock: Default::default(),
            cumulate_dividends_stock_money: Default::default(),
            cumulate_dividends_total: Default::default(),
            create_time: Default::default(),
        }
    }

    pub async fn update_cumulate_dividends(&self) -> Result<()> {

        let sql = r#"
update
    "Favorite"
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
            .bind(self.cumulate_dividends_cash)
            .bind(self.cumulate_dividends_stock)
            .bind(self.cumulate_dividends_stock_money)
            .bind(self.cumulate_dividends_total)
            .execute(&DB.pool)
            .await?;
        Ok(())
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
            number_of_shares_held: self.number_of_shares_held,
            cost_per_share: self.cost_per_share,
            cost: self.cost,
            is_sold: self.is_sold,
            cumulate_dividends_cash: self.cumulate_dividends_cash,
            cumulate_dividends_stock: self.cumulate_dividends_stock,
            cumulate_dividends_stock_money: self.cumulate_dividends_stock_money,
            cumulate_dividends_total: self.cumulate_dividends_total,
            create_time: self.create_time,
        }
    }
}

/// 取得庫存股票的數據
pub async fn fetch() -> Result<Vec<Entity>> {
    let answers = sqlx::query(
        r#"
select "Id",
       "MemberId",
       "SecurityCode",
       "NumberOfSharesHeld",
       "AverageCost",
       "CreateTime",
       "AmountPerShare",
       "IsSold",
       cumulate_dividends_cash,
       cumulate_dividends_stock,
       cumulate_dividends_stock_money,
       cumulate_dividends_total
from "Favorite"
where "IsSold" = false
        "#,
    )
    .try_map(|row: PgRow| {
        Ok(Entity {
            serial: row.try_get("Id")?,
            security_code: row.try_get("SecurityCode")?,
            member_id: row.try_get("MemberId")?,
            number_of_shares_held: row.try_get("NumberOfSharesHeld")?,
            cost_per_share: row.try_get("AmountPerShare")?,
            cost: row.try_get("AverageCost")?,
            create_time: row.try_get("CreateTime")?,
            is_sold: false,
            cumulate_dividends_cash: row.try_get("cumulate_dividends_cash")?,
            cumulate_dividends_stock: row.try_get("cumulate_dividends_stock")?,
            cumulate_dividends_stock_money: row.try_get("cumulate_dividends_stock_money")?,
            cumulate_dividends_total: row.try_get("cumulate_dividends_total")?,
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
