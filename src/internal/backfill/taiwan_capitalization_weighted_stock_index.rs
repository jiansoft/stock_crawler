use crate::internal::{bot, cache::SHARE, crawler::twse, database::model, logging};
use anyhow::*;
use chrono::Local;
use core::result::Result::Ok;

/// 調用  twse API 取得台股加權指數
pub async fn execute() -> Result<()> {
    let tai_ex = match twse::taiwan_capitalization_weighted_stock_index::visit(Local::now()).await {
        None => {
            return Err(anyhow!(
                "Failed to visit because response is no data".to_string()
            ))
        }
        Some(result) => result,
    };

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

            let index = match model::index::Entity::from_strings(&item) {
                Ok(i) => i,
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to index::Entity::from_strings because {:?}\r\n item:{:?}",
                        why, item
                    ));
                    continue;
                }
            };
            logging::debug_file_async(format!("index:{:?}", index));
            let key = index.date.to_string() + "_" + &index.category;
            if let Ok(indices) = SHARE.indices.read() {
                if indices.contains_key(key.as_str()) {
                    continue;
                }
            }

            match index.upsert().await {
                Ok(_) => {
                    logging::info_file_async(format!("index add {:?}", index));
                    let msg = format!(
                        "{} 大盤指數︰{} 漲跌︰{}",
                        index.date, index.index, index.change
                    );
                    if let Err(why) = bot::telegram::send(&msg).await {
                        logging::error_file_async(format!(
                            "Failed to telegram::send_to_allowed() because: {:?}",
                            why
                        ));
                    }
                    match SHARE.indices.write() {
                        Ok(mut indices) => {
                            indices.insert(key, index);
                        }
                        Err(why) => {
                            logging::error_file_async(format!(
                                "Failed to write stocks cache because {:?}",
                                why
                            ));
                        }
                    }
                }
                Err(why) => {
                    logging::error_file_async(format!("Failed to upsert because {:?}", why));
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::logging;

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
