use anyhow::{anyhow, Result};
use chrono::NaiveDate;

use crate::internal::database::{
    self,
    table::{
        daily_money_history::DailyMoneyHistory,
        daily_money_history_detail::DailyMoneyHistoryDetail,
        daily_money_history_detail_more::DailyMoneyHistoryDetailMore,
    },
};

/// 計算指定日期帳戶內的市值
pub async fn calculate_money_history(date: NaiveDate) -> Result<()> {
    let mut tx_option = database::get_tx().await.ok();

    if let Err(why) = DailyMoneyHistory::upsert(date, &mut tx_option).await {
        if let Some(tx) = tx_option {
            tx.rollback().await?;
        }
        return Err(anyhow!("{:?}", why));
    }

    if let Err(why) = DailyMoneyHistoryDetail::delete(date, &mut tx_option).await {
        if let Some(tx) = tx_option {
            tx.rollback().await?;
        }
        return Err(anyhow!("{:?}", why));
    }

    if let Err(why) = DailyMoneyHistoryDetail::upsert(date, &mut tx_option).await {
        if let Some(tx) = tx_option {
            tx.rollback().await?;
        }
        return Err(anyhow!("{:?}", why));
    }

    if let Err(why) = DailyMoneyHistoryDetailMore::delete(date, &mut tx_option).await {
        if let Some(tx) = tx_option {
            tx.rollback().await?;
        }
        return Err(anyhow!("{:?}", why));
    }

    if let Err(why) = DailyMoneyHistoryDetailMore::upsert(date, &mut tx_option).await {
        if let Some(tx) = tx_option {
            tx.rollback().await?;
        }
        return Err(anyhow!("{:?}", why));
    }

    if let Some(tx) = tx_option {
        tx.commit().await?;
    }

    Ok(())
}
