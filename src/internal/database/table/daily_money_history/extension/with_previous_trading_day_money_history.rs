use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;

use crate::internal::database::{self, table::daily_money_history::DailyMoneyHistory};

#[derive(sqlx::Type, sqlx::FromRow, Default, Debug)]
pub struct DailyMoneyHistoryWithPreviousTradingDayMoneyHistory {
    pub date: NaiveDate,
    pub created_at: DateTime<Local>,
    pub updated_at: DateTime<Local>,
    pub unice: Decimal,
    pub eddie: Decimal,
    pub sum: Decimal,

    pub previous_date: NaiveDate,
    pub previous_unice: Decimal,
    pub previous_eddie: Decimal,
    pub previous_sum: Decimal,
}

impl DailyMoneyHistoryWithPreviousTradingDayMoneyHistory {
    pub async fn fetch(
        date: NaiveDate,
    ) -> Result<DailyMoneyHistoryWithPreviousTradingDayMoneyHistory> {
        let sql = "
select date, sum, eddie, unice, created_time as created_at, updated_time as updated_at
from daily_money_history
where date <= $1
order by date desc
limit 2;"
            .to_string();
        let result = sqlx::query_as::<_, DailyMoneyHistory>(&sql)
            .bind(date)
            .fetch_all(database::get_connection())
            .await
            .context(format!("Failed to fetch({}) from database", date))?;

        let mut dmhwptdmh = DailyMoneyHistoryWithPreviousTradingDayMoneyHistory {
            date,
            created_at: Default::default(),
            updated_at: Default::default(),
            unice: Default::default(),
            eddie: Default::default(),
            sum: Default::default(),
            previous_date: Default::default(),
            previous_unice: Default::default(),
            previous_eddie: Default::default(),
            previous_sum: Default::default(),
        };

        for r in result {
            if r.date == date {
                dmhwptdmh.unice = r.unice;
                dmhwptdmh.eddie = r.eddie;
                dmhwptdmh.sum = r.sum;
            } else {
                dmhwptdmh.previous_unice = r.unice;
                dmhwptdmh.previous_eddie = r.eddie;
                dmhwptdmh.previous_sum = r.sum;
                dmhwptdmh.previous_date = r.date;
                break;
            }
        }

        Ok(dmhwptdmh)
    }
}

#[cfg(test)]
mod tests {
    use core::result::Result::Ok;

    use chrono::Local;

    use crate::internal::logging;

    use super::*;

    #[tokio::test]
    async fn test_fetch_stocks_with_dividends_on_date() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch".to_string());
        let d = Local::now().date_naive();
        match DailyMoneyHistoryWithPreviousTradingDayMoneyHistory::fetch(d).await {
            Ok(cd) => {
                dbg!(&cd);
                logging::debug_file_async(format!("stock: {:?}", cd));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to fetch because {:?}", why));
            }
        }

        logging::debug_file_async("結束 fetch".to_string());
    }
}
