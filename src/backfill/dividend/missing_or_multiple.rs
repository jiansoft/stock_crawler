use std::{collections::HashSet, time::Duration};

use anyhow::{Context, Result};
use chrono::Local;

use crate::{
    crawler::yahoo,
    database::table::{self, dividend},
    logging, nosql,
    util::map::Keyable,
};

/// 歷年股利批次回補的執行結果。
///
/// 此結構用於回報指定年度內有季配/半年配股票的批次回補結果，讓呼叫端可以知道本次實際處理了
/// 幾檔股票，以及總共成功 upsert 多少筆 Yahoo 股利明細。
#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoricalDividendBackfillSummary {
    /// 本次已完成歷年股利回補的股票檔數。
    pub stock_count: usize,
    /// 本次成功 upsert 的 Yahoo 股利明細筆數，不包含年度彙總列。
    pub detail_count: usize,
}

/// 回補指定年度缺少年度股利彙總，或涉及多次配息的股票。
///
/// 此流程會先建立兩份資料：
/// 1. 待採集股票清單：包含指定發放年度尚未有年度彙總列的股票，以及已有季配/半年配紀錄的股票。
/// 2. 多次配息快取：用 `security_code-year_of_dividend-quarter` 記住既有季配/半年配資料。
///
/// 多次配息查詢會同時涵蓋 `year` 與 `year_of_dividend`，因為剛跨年度時常見
/// 「去年 Q4 股利於今年發放」的資料。後續抓 Yahoo 後只處理指定年度與前一年度的
/// 股利所屬年度，並依快取排除已存在的季配/半年配紀錄，再將缺少的資料 upsert 回資料庫。
///
/// Redis 會以股票代碼建立短期快取，避免排程短時間內重複打 Yahoo。單檔股票失敗時只寫 log，
/// 不中斷整批採集。
pub(super) async fn backfill_missing_or_multiple_dividends(year: i32) -> Result<()> {
    // 先找出指定發放年度還沒有年度彙總列的股票，這批需要從 Yahoo 補出年度或近期配息資料。
    let mut stock_symbols: HashSet<String> = dividend::Dividend::fetch_no_dividends_for_year(year)
        .await?
        .into_iter()
        .collect();
    // 再找出與指定年度相關的季配/半年配資料；查詢同時看發放年度與股利所屬年度，避免跨年度漏判。
    let multiple_dividends = dividend::Dividend::fetch_multiple_dividends_for_year(year).await?;
    let mut multiple_dividend_cache = HashSet::new();
    for dividend in multiple_dividends {
        let key = dividend.key();
        // 快取 key 與 Yahoo 回補時的 key 相同，用來判斷某一季或半年配息是否已經存在。
        multiple_dividend_cache.insert(key);
        // 已有季配/半年配的股票也要重新掃描，才能補齊缺漏季度或重算年度彙總列。
        stock_symbols.insert(dividend.security_code.to_string());
    }

    logging::info_file_async(format!("本次殖利率的採集需收集 {} 家", stock_symbols.len()));
    for stock_symbol in stock_symbols {
        // Yahoo 股利回補使用獨立快取命名空間，避免和 Goodinfo 盈餘分配率快取互相影響。
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
            // 近期已處理過同一檔股票就略過，降低外部來源流量與封鎖風險。
            continue;
        }

        // 先寫入 3 天快取旗標；即使單檔處理失敗，也避免排程下一輪立刻重試同一來源。
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

        // 單檔股票的抓取、轉換或入庫失敗只記錄錯誤，整批任務繼續處理其他股票。
        if let Err(why) =
            backfill_recent_dividends_for_stock(year, &stock_symbol, &multiple_dividend_cache).await
        {
            logging::error_file_async(format!(
                "backfill_missing_or_multiple_dividends failed: year={}, stock_symbol={}, stage=backfill_recent_dividends_for_stock, error={:#}",
                year, stock_symbol, why
            ));
        }

        // 每檔股票間隔 1 秒，避免連續請求 Yahoo 造成限流或暫時封鎖。
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Ok(())
}

/// 回補單一股票在 Yahoo 可取得的歷年股利發放資料。
///
/// 此函式不套用年度篩選，也不使用 Redis 快取，適合手動修補某檔股票的歷史資料。
/// Yahoo 回傳的每一筆股利明細都會轉成 `Dividend` 並以 `upsert` 寫入資料庫；
/// 若明細是季配或半年配，最後會依涉及的發放年度重算年度彙總列。
///
/// 回傳值是本次成功 upsert 的股利明細筆數，不包含後續年度彙總列。若 Yahoo 抓取、
/// 明細 upsert 或年度彙總 upsert 任一步驟失敗，會直接回傳錯誤，讓呼叫端知道該股票回補未完成。
///
/// # 參數
///
/// - `stock_symbol`：要回補歷年股利的股票代號，例如 `2330`。
///
/// # 錯誤
///
/// Yahoo 頁面抓取或解析失敗、任一筆股利明細入庫失敗、年度彙總列入庫失敗時會回傳 `Err`。
#[cfg(test)]
pub async fn backfill_historical_dividends_for_stock(stock_symbol: &str) -> Result<usize> {
    // 歷年回補是針對單一股票的手動修補流程，因此直接打 Yahoo，不讀寫排程快取。
    let dividends_from_yahoo = yahoo::dividend::visit(stock_symbol)
        .await
        .with_context(|| format!("yahoo historical dividend fetch failed: {stock_symbol}"))?;
    // 同一發放年度可能有多筆季配/半年配，先收集年度後統一重算年度彙總，避免每筆明細都重跑聚合。
    let mut annual_total_refresh_years: HashSet<i32> = HashSet::new();
    let mut upserted_count = 0usize;

    // Yahoo 已依發放年度分組；歷年回補不篩年度，所有明細都要依來源資料寫回。
    for (paid_year, dividend_details_from_yahoo) in &dividends_from_yahoo.dividend {
        for dividend_from_yahoo in dividend_details_from_yahoo {
            // 將 Yahoo 的欄位映射成 dividend 表欄位，保留日期與每股股利數值。
            let entity = yahoo_dividend_to_entity(stock_symbol, dividend_from_yahoo);
            entity.upsert().await.with_context(|| {
                format!(
                    "historical dividend upsert failed: stock_symbol={}, paid_year={}, year_of_dividend={}, quarter={}",
                    stock_symbol, paid_year, entity.year_of_dividend, entity.quarter
                )
            })?;
            upserted_count += 1;

            if !entity.quarter.is_empty() {
                // 季配/半年配明細會影響年度彙總列，記錄其發放年度以便後續聚合。
                annual_total_refresh_years.insert(entity.year);
            }
        }
    }

    for refresh_year in annual_total_refresh_years {
        // 年度彙總列由資料庫現有季配/半年配明細聚合產生，因此 seed 只需要股票代號與發放年度。
        let mut annual_total_seed = table::dividend::Dividend::new();
        annual_total_seed.security_code = stock_symbol.to_string();
        annual_total_seed.year = refresh_year;
        annual_total_seed
            .upsert_annual_total_dividend()
            .await
            .with_context(|| {
                format!(
                    "historical annual total dividend upsert failed: stock_symbol={}, refresh_year={}",
                    stock_symbol, refresh_year
                )
            })?;
    }

    Ok(upserted_count)
}

/// 回補指定年度所有季配或半年配股票的歷年 Yahoo 股利資料。
///
/// 此函式會先呼叫 `fetch_multiple_dividends_for_year` 找出與指定年度相關的季配/半年配股利資料，
/// 再依股票代號去重，逐檔呼叫 `backfill_historical_dividends_for_stock`。每檔股票都會使用 Yahoo
/// 採集可取得的歷年股利明細，並以 `upsert` 寫回 `dividend` 表。
///
/// 批次處理時每檔股票之間會停 1 秒，降低連續請求 Yahoo 造成限流或暫時封鎖的機率。
///
/// # 參數
///
/// - `year`：要找出季配/半年配股票的指定年度。查詢會同時涵蓋發放年度與股利所屬年度。
///
/// # 錯誤
///
/// 查詢資料庫失敗、任一檔股票 Yahoo 採集失敗、明細 upsert 失敗或年度彙總 upsert 失敗時，
/// 會直接回傳 `Err`。這個函式用於手動批次修補，因此採 fail-fast，避免靜默漏補某檔股票。
#[cfg(test)]
pub async fn backfill_historical_dividends_for_multiple_dividend_stocks(
    year: i32,
) -> Result<HistoricalDividendBackfillSummary> {
    // 先取得指定年度相關的季配/半年配資料；這批資料代表需要重新用 Yahoo 歷年資料校正的股票集合。
    let multiple_dividends = dividend::Dividend::fetch_multiple_dividends_for_year(year)
        .await
        .with_context(|| format!("fetch multiple dividends failed: year={year}"))?;
    // 同一檔股票可能有多筆 Q/H 明細，批次回補只需要每檔股票跑一次 Yahoo 歷年採集。
    let stock_symbols: HashSet<String> = multiple_dividends
        .into_iter()
        .map(|dividend| dividend.security_code)
        .collect();
    let mut summary = HistoricalDividendBackfillSummary::default();

    for stock_symbol in stock_symbols {
        // 逐檔回補歷年股利；任一檔失敗就回錯，讓手動執行者能看到明確的失敗股票。
        let detail_count = backfill_historical_dividends_for_stock(&stock_symbol)
            .await
            .with_context(|| {
                format!(
                    "backfill historical dividends failed from multiple dividend stock list: year={}, stock_symbol={}",
                    year, stock_symbol
                )
            })?;
        summary.stock_count += 1;
        summary.detail_count += detail_count;

        // 批次重跑可能涵蓋多檔股票，固定節流避免短時間大量請求 Yahoo。
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Ok(summary)
}

/// 處理單一股票的近期股利抓取、去重、入庫與年度彙總更新。
///
/// 主要步驟：
/// 1. 從 Yahoo 取得該股票依發放年度分組的股利明細。
/// 2. 僅保留指定年度與前一年度的股利所屬年度，避免回寫太舊的歷史資料。
/// 3. 對季配/半年配資料使用 `multiple_dividend_cache` 排除已存在紀錄。
/// 4. 將缺少的 Yahoo 明細轉為 `Dividend` 後 upsert。
/// 5. 如果新增或更新季配/半年配資料，最後依發放年度重算年度彙總列。
///
/// # 參數
///
/// - `year`：排程要處理的年度，通常是目前本地年度。
/// - `stock_symbol`：要採集的股票代號。
/// - `multiple_dividend_cache`：既有季配/半年配資料的 key 集合，key 格式需和
///   `Dividend::key()` 一致。
///
/// # 錯誤
///
/// Yahoo 抓取失敗會回傳 `Err`，讓外層記錄該股票失敗原因；單筆資料 upsert 或年度彙總失敗
/// 只會寫 log，避免一筆 DB 異常中斷整檔股票後續資料處理。
async fn backfill_recent_dividends_for_stock(
    year: i32,
    stock_symbol: &str,
    multiple_dividend_cache: &HashSet<String>,
) -> Result<()> {
    // 先從 Yahoo 讀取單一股票的股利頁面；這一步失敗代表該股票無法繼續處理，所以直接向外回錯。
    let dividends_from_yahoo = yahoo::dividend::visit(stock_symbol)
        .await
        .with_context(|| {
            format!(
                "yahoo dividend fetch failed: year={}, stock_symbol={}",
                year, stock_symbol
            )
        })?;
    // 記錄需要重算年度彙總列的發放年度；用 HashSet 可避免同年度多個季度重複執行聚合 SQL。
    let mut annual_total_refresh_years: HashSet<i32> = HashSet::new();

    // Yahoo 回傳資料已依發放年度分組；這裡直接借用迭代，避免 clone 大量明細資料。
    for (paid_year, dividend_details_from_yahoo) in &dividends_from_yahoo.dividend {
        // 同一個 paid_year 底下可能有年度配息、季配或半年配，多筆都要逐一判斷。
        for dividend_from_yahoo in dividend_details_from_yahoo {
            // Yahoo 頁面會帶歷史資料；排程只補指定年度與前一年度，避免舊資料覆蓋現有校正結果。
            if !should_process_dividend_year(year, dividend_from_yahoo.year_of_dividend) {
                continue;
            }

            // key 使用股利所屬年度而非發放年度，才能正確區分跨年度發放的 Q4 或 H2 配息。
            let key = make_dividend_key(
                stock_symbol,
                dividend_from_yahoo.year_of_dividend,
                &dividend_from_yahoo.quarter,
            );

            if multiple_dividend_cache.contains(&key) {
                // 既有季配/半年配資料已存在時略過，避免重複 upsert 造成日期或金額被來源資料覆寫。
                continue;
            }

            // 將 Yahoo 明細映射成資料表實體；Yahoo 沒有提供的細項欄位會保留預設值。
            let entity = yahoo_dividend_to_entity(stock_symbol, dividend_from_yahoo);
            match entity.upsert().await {
                Ok(_) => {
                    logging::debug_file_async(format!(
                        "dividend upsert executed successfully. \r\n{:#?}",
                        entity
                    ));

                    if !entity.quarter.is_empty() {
                        // 只有季配/半年配需要重算年度彙總；年度配息本身就是彙總列，不需要再聚合。
                        annual_total_refresh_years.insert(entity.year);
                    }
                }
                Err(why) => {
                    // 單筆 upsert 失敗只記錄錯誤，讓同檔股票其他季度仍有機會完成入庫。
                    logging::error_file_async(format!(
                        "dividend upsert failed: year={}, paid_year={}, stock_symbol={}, year_of_dividend={}, quarter={}, error={:#}",
                        year, paid_year, stock_symbol, entity.year_of_dividend, entity.quarter, why
                    ));
                }
            }
        }
    }

    for refresh_year in annual_total_refresh_years {
        // 年度彙總 SQL 只需要股票代號與發放年度，其餘欄位由聚合查詢產生。
        let mut annual_total_seed = table::dividend::Dividend::new();
        annual_total_seed.security_code = stock_symbol.to_string();
        annual_total_seed.year = refresh_year;

        if let Err(why) = annual_total_seed.upsert_annual_total_dividend().await {
            // 年度彙總失敗不影響已寫入的季配/半年配明細，因此記錄後繼續處理下一個年度。
            logging::error_file_async(format!(
                "upsert_annual_total_dividend failed: year={}, stock_symbol={}, refresh_year={}, error={:#}",
                year, stock_symbol, refresh_year, why
            ));
        }
    }

    Ok(())
}

/// 判斷 Yahoo 股利明細是否屬於本次回補範圍。
///
/// 回補範圍限定為目標年度與前一年度的「股利所屬年度」。剛跨年度時，去年的 Q4 或 H2
/// 常會在今年發放，因此前一年度仍需納入；未來年度與更早歷史年度則略過，避免誤寫入。
fn should_process_dividend_year(target_year: i32, year_of_dividend: i32) -> bool {
    year_of_dividend == target_year || year_of_dividend == target_year - 1
}

/// 建立季配/半年配去重用的股利 key。
///
/// key 使用股票代號、股利所屬年度與季度，格式需與 `Dividend::key()` 一致。這可讓
/// Yahoo 回補資料與資料庫既有資料用同一套規則比對，即使實際發放年度跨到下一年也不會誤判。
fn make_dividend_key(stock_symbol: &str, year_of_dividend: i32, quarter: &str) -> String {
    format!("{stock_symbol}-{year_of_dividend}-{quarter}")
}

/// 建立 Yahoo 股利回補用的 Redis 快取 key。
///
/// 快取只以股票代號為粒度，目的是限制短時間內對同一 Yahoo 股利頁面的重複請求。
/// key 前綴固定為 `yahoo:dividend:`，避免與 Goodinfo 或其他資料來源的快取混用。
fn make_cache_key(stock_symbol: &str) -> String {
    format!("yahoo:dividend:{stock_symbol}")
}

/// 將 Yahoo 股利明細轉成資料庫 `Dividend` 實體。
///
/// Yahoo 來源提供發放年度、股利所屬年度、季度、現金/股票股利與日期欄位；盈餘/公積拆分與
/// 盈餘分配率在此來源沒有資料，因此沿用 `Dividend::new()` 的預設值。`sum` 由現金股利加上
/// 股票股利計算，時間欄位使用本地現在時間。
fn yahoo_dividend_to_entity(
    stock_symbol: &str,
    d: &yahoo::dividend::YahooDividendDetail,
) -> dividend::Dividend {
    // 先建立預設實體，讓 Yahoo 沒提供的盈餘/公積拆分與分配率維持 0。
    let mut e = table::dividend::Dividend::new();
    // 股票代號由呼叫端傳入，避免依賴 Yahoo 明細內部是否重複保存代號。
    e.security_code = stock_symbol.to_string();
    // `year` 是實際發放年度，會用於除權息提醒與年度彙總聚合。
    e.year = d.year;
    // `year_of_dividend` 是股利所屬年度，例如 2024Q4 可能在 2025 發放。
    e.year_of_dividend = d.year_of_dividend;
    // 季度空字串代表年度配息；Q/H 代表季配或半年配。
    e.quarter = d.quarter.clone();
    // Yahoo 已提供合計後的現金股利與股票股利，直接保留 Decimal 精度。
    e.cash_dividend = d.cash_dividend;
    e.stock_dividend = d.stock_dividend;
    // 資料表的 `sum` 為每股現金股利與每股股票股利的合計。
    e.sum = d.cash_dividend + d.stock_dividend;
    // 日期欄位可能是實際日期或 `-`；此流程照來源保存，未公布日期由另一條回補流程處理。
    e.ex_dividend_date1 = d.ex_dividend_date1.clone();
    e.ex_dividend_date2 = d.ex_dividend_date2.clone();
    e.payable_date1 = d.payable_date1.clone();
    e.payable_date2 = d.payable_date2.clone();
    // 新增與更新時間都用目前本地時間，讓後續可以追蹤本次回補寫入時間。
    e.created_time = Local::now();
    e.updated_time = Local::now();
    e
}

#[cfg(test)]
mod tests {
    use chrono::Datelike;
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
        backfill_missing_or_multiple_dividends(2025)
            .await
            .expect("backfill_missing_or_multiple_dividends failed");
    }

    #[tokio::test]
    #[ignore]
    async fn test_backfill_recent_dividends_for_stock_live() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        let year = 2026;
        let multiple_dividends = dividend::Dividend::fetch_multiple_dividends_for_year(year)
            .await
            .unwrap();
        let mut multiple_dividend_cache = HashSet::new();
        for dividend in multiple_dividends {
            multiple_dividend_cache.insert(dividend.key());
        }

        let _ = backfill_recent_dividends_for_stock(year, "6123", &multiple_dividend_cache).await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_backfill_historical_dividends_for_stock_from_yahoo_live() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        let upserted_count = backfill_historical_dividends_for_stock("5306")
            .await
            .expect("backfill historical dividends for stock failed");

        assert!(upserted_count > 0);
    }

    /// 以資料庫中指定年度所有已有季配/半年配的股票，驗證 Yahoo 歷年股利批次回補流程。
    ///
    /// 測試流程會先呼叫 `fetch_multiple_dividends_for_year` 找出目前年度已有多次配息資料的股票，
    /// 依股票代號去重後，逐檔呼叫 `backfill_historical_dividends_for_stock`。這可確認歷年回補流程
    /// 能銜接現有年度配息資料來源，並把所有有季配/半年配的股票都以 `upsert` 寫回 Yahoo
    /// 可取得的歷年股利明細。
    #[tokio::test]
    #[ignore]
    async fn test_backfill_historical_dividends_for_stock_from_multiple_dividend_year_live() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        let year = Local::now().year();
        let summary = backfill_historical_dividends_for_multiple_dividend_stocks(year)
            .await
            .unwrap_or_else(|why| {
                panic!(
                    "backfill historical dividends failed for all multiple dividend stocks in {year}: {why:#}"
                )
            });

        assert!(
            summary.stock_count > 0,
            "expected at least one multiple dividend stock for {year}"
        );
        assert!(
            summary.detail_count > 0,
            "expected Yahoo historical dividends to upsert at least one row for multiple dividend stocks in {year}"
        );
    }
}
