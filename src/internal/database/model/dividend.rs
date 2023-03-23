use chrono::{DateTime, FixedOffset, Local};
use rust_decimal::Decimal;
use anyhow::Result;
use sqlx::postgres::PgRow;
use sqlx::Row;
use crate::internal::database::DB;

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 股息發放日程表 原表名 dividend
pub struct Entity {
    /// 序號
    pub serial: i64,
    /// 發放年度
    pub year: i32,
    /// 股利所屬年度
    pub year_of_dividend: i32,
    /// 發放季度
    pub quarter: String,
    /// 股票代號
    pub security_code: String,
    /// 現金股利(元)
    pub cash: Decimal,
    /// 股票股利(元)
    pub stock: Decimal,
    /// 合計股利(元)
    pub sum: Decimal,
    /// 除息日
    pub ex_dividend_date1: String,
    /// 除權日
    pub ex_dividend_date2: String,
    /// 現金股利發放日
    pub payable_date1: String,
    /// 股票股利發放日
    pub payable_date2: String,
    pub create_time: DateTime<Local>,
    pub update_time: DateTime<Local>,
}


impl Entity {
    pub fn new() -> Self {
        Entity {
            serial: 0,
            year: 0,
            year_of_dividend: 0,
            quarter: "".to_string(),
            security_code: Default::default(),
            cash: Default::default(),
            stock: Default::default(),
            sum: Default::default(),
            ex_dividend_date1: "".to_string(),
            ex_dividend_date2: "".to_string(),
            payable_date1: "".to_string(),
            payable_date2: "".to_string(),
            create_time: Default::default(),
            update_time: Default::default(),
        }
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
            year: self.year,
            year_of_dividend: self.year_of_dividend,
            quarter: self.quarter.to_string(),
            security_code: self.security_code.to_string(),
            cash: self.cash,
            stock: self.stock,
            sum: self.sum,
            ex_dividend_date1: self.ex_dividend_date1.to_string(),
            ex_dividend_date2: self.ex_dividend_date2.to_string(),
            payable_date1: self.payable_date1.to_string(),
            payable_date2: self.payable_date2.to_string(),
            create_time: self.create_time,
            update_time: self.update_time,
        }
    }
}


/// 取得庫存股票的數據
pub async fn fetch_stock_inventory(security_code:&str,date_time: DateTime<FixedOffset>) -> Result<Vec<Entity>> {
    let answers = sqlx::query(
        r#"
select "Id",
       "MemberId",
       "SecurityCode",
       "NumberOfSharesHeld",
       "AverageCost",
       "CreateTime",
       "AmountPerShare",
       "IsSold"
from "Favorite"
where "IsSold" = false
        "#,
    )
        .bind(security_code)
        .bind(date_time)
        .try_map(|row: PgRow| {
            Ok(Entity {
                serial: row.try_get("Id")?,
                year: row.try_get("year")?,
                year_of_dividend: 0,
                quarter: "".to_string(),
                security_code: row.try_get("SecurityCode")?,

                cash: Default::default(),
                stock: Default::default(),
                sum: Default::default(),
                ex_dividend_date1: "".to_string(),
                ex_dividend_date2: "".to_string(),
                payable_date1: "".to_string(),
                payable_date2: "".to_string(),
                create_time: Default::default(),
                update_time: Default::default(),
            })
        })
        .fetch_all(&DB.pool)
        .await?;

    Ok(answers)
}
