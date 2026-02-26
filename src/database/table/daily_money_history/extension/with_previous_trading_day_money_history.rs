use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;

use crate::database::{self, table::daily_money_history::DailyMoneyHistory};

/// 當日與前一個交易日的市值對照資料。
///
/// 用於計算收盤通知中的「市值增減」與「報酬率變化」。
#[derive(sqlx::Type, sqlx::FromRow, Default, Debug)]
pub struct DailyMoneyHistoryWithPreviousTradingDayMoneyHistory {
    /// 指定查詢日期。
    pub date: NaiveDate,
    /// 當日資料建立時間。
    pub created_at: DateTime<Local>,
    /// 當日資料更新時間。
    pub updated_at: DateTime<Local>,
    /// 當日 Unice 市值。
    pub unice: Decimal,
    /// 當日 Eddie 市值。
    pub eddie: Decimal,
    /// 當日合計市值。
    pub sum: Decimal,

    /// 前一個交易日日期。
    pub previous_date: NaiveDate,
    /// 前一個交易日 Unice 市值。
    pub previous_unice: Decimal,
    /// 前一個交易日 Eddie 市值。
    pub previous_eddie: Decimal,
    /// 前一個交易日合計市值。
    pub previous_sum: Decimal,
}

impl DailyMoneyHistoryWithPreviousTradingDayMoneyHistory {
    /// 取得指定日期與前一交易日的市值資料。
    ///
    /// 內部會先抓 `date <= 指定日期` 的最近兩筆資料，
    /// 再拆成「當日」與「前一日」欄位回傳。
    ///
    /// # Errors
    /// 當資料庫查詢失敗時回傳錯誤。
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

    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_daily_money_history_with_previous_trading_day_money_history_fetch() {
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
