use anyhow::{anyhow, Result};
use chrono::NaiveDate;

use crate::database::{
    self,
    table::{
        daily_money_history::DailyMoneyHistory,
        daily_money_history_detail::DailyMoneyHistoryDetail,
        daily_money_history_detail_more::DailyMoneyHistoryDetailMore,
        daily_money_history_member::DailyMoneyHistoryMember,
        daily_stock_price_stats::DailyStockPriceStats,
    },
};

/// 計算並重建指定交易日的帳戶市值相關資料。
///
/// 這個方法會在台股收盤匯總流程
/// (`event::taiwan_stock::closing::aggregate`) 中被呼叫，並依序更新：
/// 1. `daily_money_history`：當日市值總覽（總額、Eddie、Unice）
/// 2. `daily_money_history_member`：會員垂直總覽（member_id 維度，可擴充）
/// 3. `daily_money_history_detail`：持股層級明細
/// 4. `daily_money_history_detail_more`：交易批次層級明細
/// 5. `daily_stock_price_stats`：當日全市場估值/均線統計
///
/// `daily_money_history_detail_more` 會依賴 `daily_money_history_detail`，
/// 因此順序不可顛倒，且 detail 類資料採「先刪除再重建」以避免殘留舊資料。
///
/// # Errors
/// 任一步驟失敗都會回滾 transaction（若已建立），並回傳錯誤。
pub async fn calculate_money_history(date: NaiveDate) -> Result<()> {
    // 優先使用同一筆 transaction 保障跨表一致性；
    // 若無法建立 transaction，則退化為各 SQL 自行執行。
    let mut tx_option = database::get_tx().await.ok();

    // 1) 先寫入當日市值總覽，供後續明細與通知流程使用。
    if let Err(why) = DailyMoneyHistory::upsert(date, &mut tx_option).await {
        if let Some(tx) = tx_option {
            tx.rollback().await?;
        }
        return Err(anyhow!("{:?}", why));
    }

    // 2) 寫入會員垂直總覽，保留舊表相容性的同時支援未來新會員。
    if let Err(why) = DailyMoneyHistoryMember::upsert(date, &mut tx_option).await {
        if let Some(tx) = tx_option {
            tx.rollback().await?;
        }
        return Err(anyhow!("{:?}", why));
    }

    // 3) 先清掉當日舊明細，再重建持股層級資料，避免重複與髒資料。
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

    // 4) 明細延伸表依賴 daily_money_history_detail，因此必須在其後重建。
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

    // 5) 最後更新當日市場統計，確保收盤流程可直接使用最新數據。
    if let Err(why) = DailyStockPriceStats::upsert(date, &mut tx_option).await {
        if let Some(tx) = tx_option {
            tx.rollback().await?;
        }
        return Err(anyhow!("{:?}", why));
    }

    // 以上步驟都成功才提交，確保跨表資料為同一版本。
    if let Some(tx) = tx_option {
        tx.commit().await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{cache::SHARE, logging};

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_calculate_money_history() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 calculate_money_history".to_string());
        let current_date = NaiveDate::parse_from_str("2026-04-02", "%Y-%m-%d").unwrap();
        match calculate_money_history(current_date).await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to calculate_money_history because {:?}",
                    why
                ));
            }
        }
        logging::debug_file_async("結束 calculate_money_history".to_string());
    }
}
