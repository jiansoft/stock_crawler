use anyhow::Result;
use chrono::Local;

use crate::util::map::Keyable;
use crate::{bot, cache::SHARE, crawler::twse, database::table, logging};

/// 調用  twse API 取得台股加權指數
pub async fn execute() -> Result<()> {
    let tai_ex = twse::taiwan_capitalization_weighted_stock_index::visit(Local::now()).await?;
    if tai_ex.stat.to_uppercase() != "OK" {
        logging::warn_file_async("抓取加權股價指數 Finish taiex.Stat is not ok".to_string());
        return Ok(());
    }

    if let Some(data) = tai_ex.data {
        for item in data {
            if item.len() != 6 {
                logging::error_file_async(format!("資料欄位不等於6 item:{:?}", item));
                continue;
            }

            let index = match table::index::Index::from_strings(&item) {
                Ok(i) => i,
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to index::Index::from_strings({:?}) because {:?}",
                        item, why
                    ));
                    continue;
                }
            };

            //logging::debug_file_async(format!("index:{:?}", index));
            let key = index.key();
            if SHARE.get_stock_index(&key).is_some() {
                continue;
            }

            match index.upsert().await {
                Ok(_) => {
                    logging::info_file_async(format!("index add {:?}", index));
                    let msg = format!(
                        "{} 大盤指數︰{} 漲跌︰{}",
                        index.date, index.index, index.change
                    );

                    bot::telegram::send(&msg).await;

                    SHARE.set_stock_index(key, index).await;
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to index.upsert({:#?}) because {:?}",
                        index, why
                    ));
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
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
