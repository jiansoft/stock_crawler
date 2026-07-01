#![allow(dead_code)]
use anyhow::{Context, Result};
use rust_decimal::Decimal;
use sqlx::{FromRow, postgres::PgQueryResult};

use crate::{core::util::map::Keyable, infra::database};

/// 股票除息的資料
#[derive(FromRow, Debug)]
pub struct PayoutRatioInfo {
    /// 股利資料序號（`dividend.serial`）。
    pub serial: i64,
    /// 股利所屬年度。
    pub year: i32,
    /// 股利季度（Q1/Q2/Q3/Q4/H1/H2 或空字串）。
    pub quarter: String,
    /// 股票代號。
    pub security_code: String,
    /// 現金配發率。
    pub payout_ratio_cash: Decimal,
    /// 股票配發率。
    pub payout_ratio_stock: Decimal,
    /// 合計配發率。
    pub payout_ratio: Decimal,
}

impl PayoutRatioInfo {
    /// 更新股息的盈餘分配率
    pub async fn update(&self) -> Result<PgQueryResult> {
        let sql = r#"
UPDATE
    dividend
SET
    payout_ratio_cash = $1,
    payout_ratio_stock = $2,
    payout_ratio = $3,
    updated_time = NOW()
WHERE
    serial = $4
"#;
        sqlx::query(sql)
            .bind(self.payout_ratio_cash)
            .bind(self.payout_ratio_stock)
            .bind(self.payout_ratio)
            .bind(self.serial)
            .execute(database::get_connection())
            .await
            .context(format!(
                "Failed to update_payout_ratio({:#?}) from database",
                self
            ))
    }
}

/// 取得指定日期為股利發放日的股票
pub async fn fetch_without_payout_ratio() -> Result<Vec<PayoutRatioInfo>> {
    let sql = r#"
select serial,
       security_code,
       year_of_dividend as year,
       quarter,
       payout_ratio_cash,
       payout_ratio_stock,
       payout_ratio
from dividend
where "sum" > 0 AND payout_ratio = 0 -- and security_code='2330'
    --and security_code in (select stock_symbol from stocks where stock_industry_id = 25)
    --order by random()
"#;

    sqlx::query_as::<_, PayoutRatioInfo>(sql)
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch_without_payout_ratio() from database".to_string())
}

/*pub fn vec_to_hashmap(
    entities: Vec<StockDividendPayoutRatioInfo>,
) -> HashMap<String, StockDividendPayoutRatioInfo> {
    let mut map = HashMap::new();
    for e in entities {
        let key = format!("{}-{}-{}", e.security_code, e.year, e.quarter);
        map.insert(key, e);
    }
    map
}
*/

impl Keyable for PayoutRatioInfo {
    fn key(&self) -> String {
        format!("{}-{}-{}", self.security_code, self.year, self.quarter)
    }

    fn key_with_prefix(&self) -> String {
        format!(
            "PayoutRatioInfo:{}-{}-{}",
            self.security_code, self.year, self.quarter
        )
    }
}

/*pub fn vec_2_hashmap<T: Keyable>(entities: Vec<T>) -> HashMap<String, T> {
    let mut map = HashMap::new();
    for e in entities {
        map.insert(e.key(), e);
    }
    map
}*/

#[cfg(test)]
mod tests {
    use core::result::Result::Ok;

    use crate::core::util::map::vec_to_hashmap;

    use super::*;

    #[tokio::test]
    async fn test_fetch_without_payout_ratio() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 StockDividendPayoutRatioInfo::fetch");

        match fetch_without_payout_ratio().await {
            Ok(cd) => {
                //dbg!(&cd);
                let h = vec_to_hashmap(cd);
                tracing::debug!("map: {:#?}", h);
            }
            Err(why) => {
                tracing::debug!("Failed to fetch because {:?}", why);
            }
        }

        tracing::debug!("結束 StockDividendPayoutRatioInfo::fetch");
    }
}
