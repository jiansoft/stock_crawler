use crate::internal::database::DB;
use anyhow::Result;
use chrono::NaiveDate;
use core::result::Result::Ok;
use rust_decimal::Decimal;

#[derive(sqlx::FromRow, Debug)]
/// 最後交易日股票報價數據
pub struct Entity {
    pub date: NaiveDate,
    pub security_code: String,
    /// 收盤價
    pub closing_price: Decimal,
}

impl Entity {
    pub fn new() -> Self {
        Entity {
            date: Default::default(),
            security_code: "".to_string(),
            closing_price: Default::default(),
        }
    }

    /// 取得最後交易日股票報價數據
    pub async fn fetch() -> Result<Vec<Entity>> {
        Ok(sqlx::query_as::<_, Entity>(
            r#"
select
    date, security_code, closing_price
from
    last_daily_quotes
"#,
        )
            .fetch_all(&DB.pool)
            .await?)
    }

    pub fn clone(&self) -> Self {
        Entity {
            date: self.date,
            security_code: self.security_code.clone(),
            closing_price: self.closing_price,
        }
    }
}

impl Default for Entity {
    fn default() -> Self {
        Self::new()
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::logging;

    #[tokio::test]
    async fn test_calculate() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 fetch".to_string());
        let _ = Entity::new();
        match Entity::fetch().await {
            Ok(stocks) => logging::info_file_async(format!("{:#?}", stocks)),
            Err(why) => {
                logging::error_file_async(format!("Failed to fetch because {:?}", why));
            }
        }

        logging::info_file_async("結束 fetch_last_trading_day_quotes".to_string());
    }
}
