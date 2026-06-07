use anyhow::Result;
use chrono::{Local, NaiveDate, TimeZone};

use crate::app::backfill::acl::IndexAclMapper;
use crate::app::event::handlers::get_global_dispatcher;
use crate::domain::events::DomainEvent;
use crate::{core::logging, infra::cache::SHARE, infra::crawler::twse};

pub async fn execute() -> Result<()> {
    let tai_ex = twse::taiwan_capitalization_weighted_stock_index::visit(Local::now()).await?;
    if tai_ex.stat.to_uppercase() != "OK" {
        logging::warn_file_async("抓取加權股價指數 Finish taiex.Stat is not ok".to_string());
        return Ok(());
    }

    if let Some(data) = tai_ex.data {
        for item in data {
            let cmd = match IndexAclMapper::from_strings(&item) {
                Some(c) => c,
                None => continue,
            };

            let key = format!("{}-TAIEX", cmd.date);
            if SHARE.get_stock_index(&key).is_some() {
                continue;
            }

            let index_entity = IndexAclMapper::from_command(&cmd);

            match index_entity.upsert().await {
                Ok(_) => {
                    logging::info_file_async(format!("index add {:?}", index_entity));

                    // 派發領域事件以發送 Telegram 通知
                    let event = DomainEvent::StockIndexUpdated {
                        date: cmd.date,
                        index: cmd.index,
                        change: cmd.change,
                        occurred_at: Local::now(),
                    };
                    get_global_dispatcher().dispatch_async(vec![event]).await;

                    SHARE.set_stock_index(key, index_entity).await;
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to index.upsert({:#?}) because {:?}",
                        index_entity, why
                    ));
                }
            }
        }
    }

    Ok(())
}

/// 依指定日期調用 TWSE API 回補該日的台股加權指數。
///
/// TWSE API 會回傳整個月份的資料，但此函式只會 upsert 與 `date` 完全相符的那一筆。
///
/// 與 [`execute`] 的差異：
/// 1. 使用呼叫端傳入的 `date` 查詢 TWSE。
/// 2. 只寫入指定日期的資料，忽略同月份其他交易日。
/// 3. 跳過 `SHARE` 快取檢查，確保資料一定會 upsert 到資料庫。
/// 4. 不發送 Telegram 通知，避免歷史回補產生誤導訊息。
///
/// 回傳成功 upsert 的資料筆數（0 或 1）。
pub async fn execute_for_date(date: NaiveDate) -> Result<usize> {
    // 將 NaiveDate 轉成 DateTime<Local>，TWSE API 只看日期中的年月部分。
    let datetime = Local
        .from_local_datetime(&date.and_hms_opt(12, 0, 0).unwrap())
        .single()
        .unwrap_or_else(Local::now);

    let tai_ex = twse::taiwan_capitalization_weighted_stock_index::visit(datetime).await?;
    if tai_ex.stat.to_uppercase() != "OK" {
        logging::warn_file_async(format!(
            "抓取加權股價指數 (date={date}) Finish taiex.Stat is not ok"
        ));
        return Ok(0);
    }

    let mut upserted_count: usize = 0;

    if let Some(data) = tai_ex.data {
        for item in data {
            let cmd = match IndexAclMapper::from_strings(&item) {
                Some(c) => c,
                None => continue,
            };

            // TWSE 回傳整月資料，只處理與指定日期相符的那一筆。
            if cmd.date != date {
                continue;
            }

            let index_entity = IndexAclMapper::from_command(&cmd);

            match index_entity.upsert().await {
                Ok(_) => {
                    logging::info_file_async(format!("index upsert (backfill) {:?}", index_entity));
                    let key = format!("{}-TAIEX", index_entity.date);
                    SHARE.set_stock_index(key, index_entity).await;
                    upserted_count += 1;
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to index.upsert({:#?}) because {:?}",
                        index_entity, why
                    ));
                }
            }
        }
    }

    Ok(upserted_count)
}

#[cfg(test)]
mod tests {
    use crate::core::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute".to_string());

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        logging::debug_file_async("結束 execute".to_string());
    }
}
