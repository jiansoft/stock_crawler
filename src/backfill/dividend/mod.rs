use std::{collections::HashSet, time::Duration};

use anyhow::{anyhow, Result};
use chrono::{Datelike, Local};
use scopeguard::defer;
use tokio_retry::{
    strategy::{jitter, ExponentialBackoff},
    Retry,
};

use crate::{
    crawler::yahoo,
    database::table::{self, dividend},
    logging, nosql,
    util::map::Keyable,
};

pub mod payout_ratio;

/// 更新股利發送數據
/// 資料庫內尚未有年度配息數據的股票取出後向第三方查詢後更新回資料庫
pub async fn execute() -> Result<()> {
    logging::info_file_async("更新台股股利發放數據開始");
    defer! {
       logging::info_file_async("更新台股股利發放數據結束");
    }

    let now = Local::now();
    let year = now.year();
    let no_or_multiple_dividend = processing_no_or_multiple(year);
    let yahoo = processing_unannounced_ex_dividend_date(year);
    let (res_no_or_multiple, res_yahoo) = tokio::join!(no_or_multiple_dividend, yahoo);

    match res_no_or_multiple {
        Ok(_) => {
            logging::info_file_async(
                "processing_without_or_multiple executed successfully.".to_string(),
            );
        }
        Err(why) => {
            logging::error_file_async(format!(
                "Failed to process_without_or_multiple because {:?}",
                why
            ));
        }
    }

    match res_yahoo {
        Ok(_) => {
            logging::info_file_async(
                "processing_with_unannounced_ex_dividend_dates executed successfully.".to_string(),
            );
        }
        Err(why) => {
            logging::error_file_async(format!("Failed to process_yahoo because {:?}", why));
        }
    }

    Ok(())
}

/// 非同步處理「今年尚未有股利資料」或「今年有多次配息」的股票。
///
/// 此函式會先取得符合條件的股票代碼清單，接著透過 `yahoo::dividend::visit`
/// 抓取各股票的股利資訊。對於每筆資料，若其股利所屬年度不是今年或去年則略過；
/// 否則轉換為 `table::dividend::Dividend` 後執行 upsert。
///
/// 當 upsert 成功時會記錄成功訊息與資料內容；失敗時則記錄錯誤訊息。
/// 為避免短時間大量請求，處理每檔股票後會暫停一段時間再繼續下一檔。
///
/// # 回傳
///
/// - `Ok(())`：全部股票處理完成
/// - `Err(e)`：處理過程中發生錯誤
///
/// # 錯誤
///
/// 此函式可能在以下情境回傳錯誤：
/// - 取得股票清單失敗
/// - 抓取個股股利資料失敗
/// - upsert 股利資料失敗
async fn processing_no_or_multiple(year: i32) -> Result<()> {
    //年度內尚未有股利配息資料
    let mut stock_symbols: HashSet<String> = dividend::Dividend::fetch_no_dividends_for_year(year)
        .await?
        .into_iter()
        .collect();
    //年度內有多次配息資料
    let multiple_dividends = dividend::Dividend::fetch_multiple_dividends_for_year(year).await?;
    let mut multiple_dividend_cache = HashSet::new();
    for dividend in multiple_dividends {
        let key = dividend.key();
        multiple_dividend_cache.insert(key);
        stock_symbols.insert(dividend.security_code.to_string());
    }

    logging::info_file_async(format!("本次殖利率的採集需收集 {} 家", stock_symbols.len()));
    for stock_symbol in stock_symbols {
        // 與 goodinfo 分開快取命名空間，避免資料來源切換時誤用舊快取。
        let cache_key = format!("yahoo:dividend:{}", stock_symbol);
        let is_jump = nosql::redis::CLIENT.get_bool(&cache_key).await?;

        if is_jump {
            continue;
        }

        nosql::redis::CLIENT
            .set(cache_key, true, 60 * 60 * 24 * 3)
            .await?;

        if let Err(why) =
            process_stock_dividends(year, &stock_symbol, &multiple_dividend_cache).await
        {
            logging::error_file_async(format!("{:?} ", why));
        }

        tokio::time::sleep(Duration::from_secs(120)).await;
    }

    Ok(())
}

/// 處理單一股票的股利資料抓取與入庫流程。
///
/// 主要步驟：
/// 1. 從 Yahoo 取得該股票的股利明細
/// 2. 僅保留今年與去年股利所屬年度的資料
/// 3. 依既有 key 規則排除已存在的多次配息紀錄
/// 4. 轉為資料表實體後 upsert，必要時更新年度總和
async fn process_stock_dividends(
    year: i32,
    stock_symbol: &str,
    multiple_dividend_cache: &HashSet<String>,
) -> Result<()> {
    // 以單一股票為處理單位，從 Yahoo 取得股利資料後寫回資料庫。
    let dividends_from_yahoo = yahoo::dividend::visit(stock_symbol).await?;
    let last_year = year - 1;
    // Yahoo 回傳依發放年度分組；這裡先展平成明細，再用股利所屬年度過濾目標資料。
    let dividend_details_from_yahoo = dividends_from_yahoo
        .dividend
        .iter()
        .flat_map(|(_, details)| details.iter().cloned())
        .collect::<Vec<_>>();

    for dividend_from_yahoo in dividend_details_from_yahoo {
        // 僅處理今年與去年的股利所屬年度，避免將過舊資料覆寫回資料庫。
        if dividend_from_yahoo.year_of_dividend != year
            && dividend_from_yahoo.year_of_dividend != last_year
        {
            continue;
        }

        // key 格式需與 `Dividend::key()` 一致，才能沿用既有多次配息去重邏輯。
        let key = format!(
            "{}-{}-{}",
            stock_symbol, dividend_from_yahoo.year_of_dividend, dividend_from_yahoo.quarter
        );

        if multiple_dividend_cache.contains(&key) {
            continue;
        }

        let entity = yahoo_dividend_to_entity(stock_symbol, &dividend_from_yahoo);
        match entity.upsert().await {
            Ok(_) => {
                logging::debug_file_async(format!(
                    "dividend upsert executed successfully. \r\n{:#?}",
                    entity
                ));

                if !entity.quarter.is_empty() {
                    // 季配/半年配資料入庫後，順便更新該年度總和（quarter = ''）。
                    if let Err(why) = entity.upsert_annual_total_dividend().await {
                        logging::error_file_async(format!("{:?} ", why));
                    }
                }
            }
            Err(why) => {
                logging::error_file_async(format!("{:?} ", why));
            }
        }
    }

    Ok(())
}

fn yahoo_dividend_to_entity(
    stock_symbol: &str,
    d: &yahoo::dividend::YahooDividendDetail,
) -> table::dividend::Dividend {
    // Yahoo 來源目前只提供股利總額與日期欄位，其餘細項沿用預設值。
    let mut e = table::dividend::Dividend::new();
    e.security_code = stock_symbol.to_string();
    e.year = d.year;
    e.year_of_dividend = d.year_of_dividend;
    e.quarter = d.quarter.clone();
    e.cash_dividend = d.cash_dividend;
    e.stock_dividend = d.stock_dividend;
    // 資料表的 `sum` 為現金股利與股票股利總和。
    e.sum = d.cash_dividend + d.stock_dividend;
    e.ex_dividend_date1 = d.ex_dividend_date1.clone();
    e.ex_dividend_date2 = d.ex_dividend_date2.clone();
    e.payable_date1 = d.payable_date1.clone();
    e.payable_date2 = d.payable_date2.clone();
    e.created_time = Local::now();
    e.updated_time = Local::now();
    e
}

/// 處理除息日為尚未公布的股票
async fn processing_unannounced_ex_dividend_date(year: i32) -> Result<()> {
    //除息日 尚未公布
    let dividends =
        dividend::Dividend::fetch_unpublished_dividend_date_or_payable_date_for_specified_year(
            year,
        )
        .await?;

    logging::info_file_async(format!(
        "本次除息日與發放日的採集需收集 {} 家",
        dividends.len()
    ));

    for dividend in dividends {
        if let Err(why) = processing_unannounced_ex_dividend_date_from_yahoo(dividend, year).await {
            logging::error_file_async(format!(
                "Failed to fetch_dividend_from_yahoo because {:?}",
                why
            ));
        }
    }
    /*let tasks: Vec<_> = dividends
        .into_iter()
        .map(|d| processing_unannounced_ex_dividend_date_from_yahoo(d, year))
        .collect();
    let results = futures::future::join_all(tasks).await;

    for result in results {
        if let Err(why) = result {
            logging::error_file_async(format!(
                "Failed to fetch_dividend_from_yahoo because {:?}",
                why
            ));
        }
    }*/

    Ok(())
}

/// 從雅虎取得除息日的資料
async fn processing_unannounced_ex_dividend_date_from_yahoo(
    mut entity: dividend::Dividend,
    year: i32,
) -> Result<()> {
    let strategy = ExponentialBackoff::from_millis(100)
        .map(jitter) // add jitter to delays
        .take(5); // limit to 5 retries
    let retry_future = Retry::spawn(strategy, || yahoo::dividend::visit(&entity.security_code));
    let yahoo = match retry_future.await {
        Ok(yahoo_dividend) => yahoo_dividend,
        Err(why) => {
            return Err(anyhow!("{}", why));
        }
    };

    // 取得今年度的股利數據
    if let Some(yahoo_dividend_details) = yahoo.get_dividend_by_year(year) {
        let yahoo_dividend_detail = yahoo_dividend_details.iter().find(|detail| {
            detail.year_of_dividend == entity.year_of_dividend
                && detail.quarter == entity.quarter
                && (detail.ex_dividend_date1 != entity.ex_dividend_date1
                    || detail.ex_dividend_date2 != entity.ex_dividend_date2
                    || detail.payable_date1 != entity.payable_date1
                    || detail.payable_date2 != entity.payable_date2)
        });

        if let Some(yahoo_dividend_detail) = yahoo_dividend_detail {
            entity.ex_dividend_date1 = yahoo_dividend_detail.ex_dividend_date1.to_string();
            entity.ex_dividend_date2 = yahoo_dividend_detail.ex_dividend_date2.to_string();
            entity.payable_date1 = yahoo_dividend_detail.payable_date1.to_string();
            entity.payable_date2 = yahoo_dividend_detail.payable_date2.to_string();

            if let Err(why) = entity.update_dividend_date().await {
                return Err(anyhow!("{}", why));
            }

            logging::info_file_async(format!(
                "dividend update_dividend_date executed successfully. \r\n{:?}",
                entity
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::cache::SHARE;

    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_processing_with_unannounced_ex_dividend_dates() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 processing_with_unannounced_ex_dividend_dates".to_string());

        match processing_unannounced_ex_dividend_date(2025).await {
            Ok(_) => {
                logging::debug_file_async(
                    "processing_with_unannounced_ex_dividend_dates executed successfully."
                        .to_string(),
                );
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to processing_with_unannounced_ex_dividend_dates because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 processing_with_unannounced_ex_dividend_dates".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_processing_without_or_multiple() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        match processing_no_or_multiple(2024).await {
            Ok(_) => {
                logging::debug_file_async(
                    "processing_without_or_multiple executed successfully.".to_string(),
                );
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to processing_without_or_multiple because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 processing_without_or_multiple".to_string());
    }

    #[tokio::test]

    async fn test_process_stock_dividends() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 process_stock_dividends".to_string());
        let year = 2024;
        let multiple_dividends = dividend::Dividend::fetch_multiple_dividends_for_year(year)
            .await
            .unwrap();
        let mut multiple_dividend_cache = HashSet::new();
        for dividend in multiple_dividends {
            let key = dividend.key();
            multiple_dividend_cache.insert(key);
        }

        match process_stock_dividends(year, "2454", &multiple_dividend_cache).await {
            Ok(_) => {
                logging::debug_file_async(
                    "process_stock_dividends executed successfully.".to_string(),
                );
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to process_stock_dividends because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 process_stock_dividends".to_string());
    }
}
