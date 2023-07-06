use crate::internal::database;
use anyhow::*;
use rust_decimal::Decimal;
use sqlx::{postgres::PgRow, Postgres, Row, Transaction};
use std::result::Result::Ok;

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 持股中積累領取的股利
pub struct CumulateDividend {
    pub stock_ownership_details_serial: i64,
    /// 累積現金股利(元)
    pub cash: Decimal,
    /// 累積股票股利(股)
    pub stock: Decimal,
    /// 累積股票股利(元)
    pub stock_money: Decimal,
    /// 累積合計股利(元)
    pub total: Decimal,
}

impl CumulateDividend {
    pub fn new(
        stock_ownership_details_serial: i64,
        cash: Decimal,
        stock_money: Decimal,
        stock: Decimal,
        total: Decimal,
    ) -> Self {
        CumulateDividend {
            stock_ownership_details_serial,
            cash,
            stock,
            stock_money,
            total,
        }
    }

    /// 計算指定股票其累積的領取股利
    pub async fn fetch_cumulate_dividend(
        stock_ownership_details_serial: i64,
        tx: &mut Option<Transaction<'_, Postgres>>,
    ) -> Result<CumulateDividend> {
        let query = sqlx::query(
            r#"
select
    COALESCE(sum(cash), 0)        as cash,
    COALESCE(sum(stock_money), 0) as stock_money,
    COALESCE(sum(stock), 0)       as stock,
    COALESCE(sum(total), 0)       as total
from dividend_record_detail
where stock_ownership_details_serial = $1;
"#,
        )
        .bind(stock_ownership_details_serial)
        .try_map(|row: PgRow| {
            let cd = CumulateDividend::new(
                stock_ownership_details_serial,
                row.try_get("cash")?,
                row.try_get("stock_money")?,
                row.try_get("stock")?,
                row.try_get("total")?,
            );
            Ok(cd)
        });

        let cd = match tx {
            None => query.fetch_one(database::get_pool()?).await?,
            Some( t) => query.fetch_one(&mut **t).await?,
        };

        Ok(cd)
    }
}
