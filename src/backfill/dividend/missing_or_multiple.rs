use std::{collections::HashSet, time::Duration};

use anyhow::{Context, Result};
use chrono::Local;

use crate::{
    crawler::yahoo,
    database::table::{self, dividend},
    logging, nosql,
    util::map::Keyable,
};

/// 非同步處理「今年尚未有股利資料」或「今年有多次配息」的股票。
///
/// 此函式會先取得符合條件的股票代碼清單，接著透過 `yahoo::dividend::visit`
/// 抓取各股票的股利資訊。對於每筆資料，若其股利所屬年度不是今年或去年則略過；
/// 否則轉換為 `table::dividend::Dividend` 後執行 upsert。
///
/// 當 upsert 成功時會記錄成功訊息與資料內容；失敗時則記錄錯誤訊息。
/// 為避免短時間大量請求，處理每檔股票後會暫停一段時間再繼續下一檔。
pub(super) async fn backfill_missing_or_multiple_dividends(year: i32) -> Result<()> {
    // 先抓「當年度沒有任何股利資料」的股票，這批是主要回補目標。
    let mut stock_symbols: HashSet<String> = dividend::Dividend::fetch_no_dividends_for_year(year)
        .await?
        .into_iter()
        .collect();
    // 再抓「當年度已存在多筆配息」的股票，這批需要依 key 規則做去重回補。
    let multiple_dividends = dividend::Dividend::fetch_multiple_dividends_for_year(year).await?;
    let mut multiple_dividend_cache = HashSet::new();
    for dividend in multiple_dividends {
        let key = dividend.key();
        // cache 用於後續排除已存在的多次配息紀錄，避免重複 upsert。
        multiple_dividend_cache.insert(key);
        // 同時把股票代碼併入待處理集合，確保兩種來源都會被掃到。
        stock_symbols.insert(dividend.security_code.to_string());
    }

    logging::info_file_async(format!("本次殖利率的採集需收集 {} 家", stock_symbols.len()));
    for stock_symbol in stock_symbols {
        // 與 goodinfo 分開快取命名空間，避免資料來源切換時誤用舊快取。
        let cache_key = make_cache_key(&stock_symbol);
        let is_jump = nosql::redis::CLIENT
            .get_bool(&cache_key)
            .await
            .with_context(|| {
                format!(
                    "redis get_bool failed: year={}, stock_symbol={}, cache_key={}",
                    year, stock_symbol, cache_key
                )
            })?;

        if is_jump {
            // 已在近期處理過就略過，避免短時間重複打外部來源。
            continue;
        }

        // 先寫入短期快取旗標（3 天），即使單檔失敗也避免立即重試造成壓力。
        nosql::redis::CLIENT
            .set(cache_key, true, 60 * 60 * 24 * 3)
            .await
            .with_context(|| {
                format!(
                    "redis set failed: year={}, stock_symbol={}, ttl_seconds={}",
                    year,
                    stock_symbol,
                    60 * 60 * 24 * 3
                )
            })?;

        // 單檔失敗只記錄錯誤不中斷整體，確保批次任務能持續推進。
        if let Err(why) =
            backfill_recent_dividends_for_stock(year, &stock_symbol, &multiple_dividend_cache)
                .await
        {
            logging::error_file_async(format!(
                "backfill_missing_or_multiple_dividends failed: year={}, stock_symbol={}, stage=backfill_recent_dividends_for_stock, error={:#}",
                year, stock_symbol, why
            ));
        }

        // 主動節流，降低被來源站台限流或封鎖的風險。
        tokio::time::sleep(Duration::from_secs(3)).await;
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
async fn backfill_recent_dividends_for_stock(
    year: i32,
    stock_symbol: &str,
    multiple_dividend_cache: &HashSet<String>,
) -> Result<()> {
    // 以單一股票為處理單位，從 Yahoo 取得股利資料後寫回資料庫。
    let dividends_from_yahoo = yahoo::dividend::visit(stock_symbol)
        .await
        .with_context(|| {
            format!(
                "yahoo dividend fetch failed: year={}, stock_symbol={}",
                year, stock_symbol
            )
        })?;
    // 同一股票同一發放年度只需聚合一次年度總和，避免每個季度都重複執行聚合 SQL。
    let mut annual_total_refresh_years: HashSet<i32> = HashSet::new();

    // 直接遍歷 Yahoo 分組資料，避免先 clone + collect 造成額外記憶體與拷貝成本。
    for (_, dividend_details_from_yahoo) in &dividends_from_yahoo.dividend {
        for dividend_from_yahoo in dividend_details_from_yahoo {
            // 僅處理今年與去年的股利所屬年度，避免將過舊資料覆寫回資料庫。
            if !should_process_dividend_year(year, dividend_from_yahoo.year_of_dividend) {
                continue;
            }

            // key 格式需與 `Dividend::key()` 一致，才能沿用既有多次配息去重邏輯。
            let key = make_dividend_key(
                stock_symbol,
                dividend_from_yahoo.year_of_dividend,
                &dividend_from_yahoo.quarter,
            );

            if multiple_dividend_cache.contains(&key) {
                continue;
            }

            let entity = yahoo_dividend_to_entity(stock_symbol, dividend_from_yahoo);
            match entity.upsert().await {
                Ok(_) => {
                    logging::debug_file_async(format!(
                        "dividend upsert executed successfully. \r\n{:#?}",
                        entity
                    ));

                    if !entity.quarter.is_empty() {
                        annual_total_refresh_years.insert(entity.year);
                    }
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "dividend upsert failed: year={}, stock_symbol={}, year_of_dividend={}, quarter={}, error={:#}",
                        year, stock_symbol, entity.year_of_dividend, entity.quarter, why
                    ));
                }
            }
        }
    }

    for refresh_year in annual_total_refresh_years {
        let mut annual_total_seed = table::dividend::Dividend::new();
        annual_total_seed.security_code = stock_symbol.to_string();
        annual_total_seed.year = refresh_year;

        if let Err(why) = annual_total_seed.upsert_annual_total_dividend().await {
            logging::error_file_async(format!(
                "upsert_annual_total_dividend failed: year={}, stock_symbol={}, refresh_year={}, error={:#}",
                year, stock_symbol, refresh_year, why
            ));
        }
    }

    Ok(())
}

fn should_process_dividend_year(target_year: i32, year_of_dividend: i32) -> bool {
    year_of_dividend == target_year || year_of_dividend == target_year - 1
}

fn make_dividend_key(stock_symbol: &str, year_of_dividend: i32, quarter: &str) -> String {
    format!("{stock_symbol}-{year_of_dividend}-{quarter}")
}

fn make_cache_key(stock_symbol: &str) -> String {
    format!("yahoo:dividend:{stock_symbol}")
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

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::cache::SHARE;

    #[test]
    fn test_should_process_dividend_year() {
        let target_year = 2025;
        assert!(should_process_dividend_year(target_year, 2025));
        assert!(should_process_dividend_year(target_year, 2024));
        assert!(!should_process_dividend_year(target_year, 2023));
        assert!(!should_process_dividend_year(target_year, 2026));
    }

    #[test]
    fn test_make_dividend_key() {
        assert_eq!(make_dividend_key("2454", 2024, "Q4"), "2454-2024-Q4");
    }

    #[test]
    fn test_yahoo_dividend_to_entity_maps_fields_and_sum() {
        let d = yahoo::dividend::YahooDividendDetail {
            year: 2025,
            year_of_dividend: 2024,
            quarter: "Q4".to_string(),
            cash_dividend: dec!(3.5),
            stock_dividend: dec!(0.2),
            ex_dividend_date1: "2025-07-01".to_string(),
            ex_dividend_date2: "2025-07-02".to_string(),
            payable_date1: "2025-08-01".to_string(),
            payable_date2: "2025-08-02".to_string(),
        };

        let e = yahoo_dividend_to_entity("2454", &d);
        assert_eq!(e.security_code, "2454");
        assert_eq!(e.year, 2025);
        assert_eq!(e.year_of_dividend, 2024);
        assert_eq!(e.quarter, "Q4");
        assert_eq!(e.cash_dividend, dec!(3.5));
        assert_eq!(e.stock_dividend, dec!(0.2));
        assert_eq!(e.sum, dec!(3.7));
        assert_eq!(e.ex_dividend_date1, "2025-07-01");
        assert_eq!(e.ex_dividend_date2, "2025-07-02");
        assert_eq!(e.payable_date1, "2025-08-01");
        assert_eq!(e.payable_date2, "2025-08-02");
    }

    #[tokio::test]
    #[ignore]
    async fn test_backfill_missing_or_multiple_dividends_live() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        let _ = backfill_missing_or_multiple_dividends(2024).await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_backfill_recent_dividends_for_stock_live() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        let year = 2024;
        let multiple_dividends = dividend::Dividend::fetch_multiple_dividends_for_year(year)
            .await
            .unwrap();
        let mut multiple_dividend_cache = HashSet::new();
        for dividend in multiple_dividends {
            multiple_dividend_cache.insert(dividend.key());
        }

        let _ =
            backfill_recent_dividends_for_stock(year, "2454", &multiple_dividend_cache).await;
    }
}
