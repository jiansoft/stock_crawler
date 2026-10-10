//! Yahoo 歷年股利的手動回補（單檔、指定年度所有季配/半年配股票）。
//!
//! 與排程用的近期補抓（`missing_or_multiple`）不同：這裡不篩年度、不讀寫 Redis 跳過旗標，
//! 也不限制單輪檔數，由後台或 `manual_backfill` 手動觸發。

use std::{collections::HashSet, time::Duration};

use anyhow::{Context, Result};
use rand::RngExt;

use crate::{
    app::backfill::acl::YahooDividendAclMapper,
    app::calculation::dividend_record,
    domain::dividend::repository::DividendRepository,
    infra::crawler::yahoo::{self, dividend::YahooDividend},
    infra::database::repository::dividend::PgDividendRepository,
};

/// 歷年股利批次回補的執行結果。
///
/// 此結構用於回報指定年度內有季配/半年配股票的批次回補結果，讓呼叫端可以知道本次實際處理了
/// 幾檔股票，以及總共成功 upsert 多少筆 Yahoo 股利明細。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoricalDividendBackfillSummary {
    /// 本次已完成歷年股利回補的股票檔數。
    pub stock_count: usize,
    /// 本次成功 upsert 的 Yahoo 股利明細筆數，不包含年度彙總列。
    pub detail_count: usize,
}

/// 回補單一股票在 Yahoo 可取得的歷年股利發放資料。
///
/// 此函式不套用年度篩選，也不使用 Redis 快取，適合手動修補某檔股票的歷史資料。
/// Yahoo 回傳的每一筆股利明細都會轉成 `Dividend` 並以 `upsert` 寫入資料庫；
/// 若明細是季配或半年配，最後會依涉及的發放年度重算年度彙總列。
/// 股利資料全部寫入完成後，會接著依股票代號回補目前持股的已領股利總表與逐項明細，
/// 避免事後補進的股利資料沒有同步反映到持股領取紀錄。
///
/// 回傳值是本次成功 upsert 的股利明細筆數，不包含後續年度彙總列。若 Yahoo 抓取、
/// 明細 upsert、年度彙總 upsert 或持股已領股利回補任一步驟失敗，會直接回傳錯誤，
/// 讓呼叫端知道該股票回補未完成。
///
/// # 參數
///
/// - `stock_symbol`：要回補歷年股利的股票代號，例如 `2330`。
///
/// # 錯誤
///
/// Yahoo 頁面抓取或解析失敗、任一筆股利明細入庫失敗、年度彙總列入庫失敗、
/// 或持股已領股利紀錄回補失敗時會回傳 `Err`。
pub async fn backfill_historical_dividends_for_stock(stock_symbol: &str) -> Result<usize> {
    // 歷年回補是針對單一股票的手動修補流程，因此直接打 Yahoo，不讀寫排程快取。
    let dividends_from_yahoo = yahoo::dividend::visit(stock_symbol)
        .await
        .with_context(|| format!("yahoo historical dividend fetch failed: {stock_symbol}"))?;
    save_historical_dividends(stock_symbol, &dividends_from_yahoo).await
}

/// 把 Yahoo 歷年股利明細全部寫回資料庫，並重算年度彙總列與持股已領股利。
///
/// 從 [`backfill_historical_dividends_for_stock`] 拆出的資料庫寫入段，不碰網路，
/// 整合測試可以直接餵組好的 [`YahooDividend`] 驗證。回傳 upsert 的明細筆數。
async fn save_historical_dividends(
    stock_symbol: &str,
    dividends_from_yahoo: &YahooDividend,
) -> Result<usize> {
    let dividend_repo = PgDividendRepository::new();
    // 同一發放年度可能有多筆季配/半年配，先收集年度後統一重算年度彙總，避免每筆明細都重跑聚合。
    let mut annual_total_refresh_years: HashSet<i32> = HashSet::new();
    let mut upserted_count = 0usize;

    // Yahoo 已依發放年度分組；歷年回補不篩年度，所有明細都要依來源資料寫回。
    for (paid_year, dividend_details_from_yahoo) in &dividends_from_yahoo.dividend {
        for dividend_from_yahoo in dividend_details_from_yahoo {
            let cmd = YahooDividendAclMapper::from_dto(stock_symbol, dividend_from_yahoo);
            let entity = YahooDividendAclMapper::from_command(&cmd);
            dividend_repo.save(&entity).await.with_context(|| {
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
        dividend_repo.upsert_annual_total_dividend(stock_symbol, refresh_year)
            .await
            .with_context(|| {
                format!(
                    "historical annual total dividend upsert failed: stock_symbol={}, refresh_year={}",
                    stock_symbol, refresh_year
                )
            })?;
    }

    // 歷史股利回補完成後，立即同步目前持股的已領股利總表與逐項明細。
    dividend_record::backfill_received_dividend_records_for_stock(stock_symbol)
        .await
        .with_context(|| {
            format!(
                "historical received dividend record backfill failed: stock_symbol={stock_symbol}"
            )
        })?;

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
pub async fn backfill_historical_dividends_for_multiple_dividend_stocks(
    year: i32,
) -> Result<HistoricalDividendBackfillSummary> {
    let stock_symbols = multiple_dividend_symbols(year).await?;
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

        // 每檔股票請求完成後，進行隨機 1.5 到 3.0 秒的延遲（Jitter），降低規律請求被 Yahoo WAF 偵測為爬蟲的機率
        let jitter_ms = rand::rng().random_range(1500..=3000);
        tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
    }

    Ok(summary)
}

/// 找出與指定年度相關、有季配/半年配的股票代號。
///
/// 這批股票需要重新用 Yahoo 歷年資料校正；同一檔可能有多筆 Q/H 明細，
/// 批次回補只需要每檔跑一次，所以回傳去重後的集合。
async fn multiple_dividend_symbols(year: i32) -> Result<HashSet<String>> {
    let multiple_dividends = PgDividendRepository::new()
        .fetch_multiple_dividends_for_year(year)
        .await
        .with_context(|| format!("fetch multiple dividends failed: year={year}"))?;

    Ok(multiple_dividends
        .into_iter()
        .map(|dividend| dividend.security_code)
        .collect())
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::infra::{crawler::yahoo::dividend::YahooDividendDetail, database};

    /// 測試用假代號；固定用歷史年度，跑完清掉。
    const TEST_SYMBOL: &str = "79969";
    /// 測試資料的發放年度。
    const TEST_PAYOUT_YEAR: i32 = 2020;

    async fn cleanup() {
        sqlx::query("DELETE FROM dividend WHERE security_code = $1")
            .bind(TEST_SYMBOL)
            .execute(database::get_connection())
            .await
            .expect("清除測試股利失敗");
    }

    fn quarter_detail(quarter: &str, cash_dividend: Decimal, ex_date: &str) -> YahooDividendDetail {
        YahooDividendDetail {
            year: TEST_PAYOUT_YEAR,
            year_of_dividend: TEST_PAYOUT_YEAR - 1,
            quarter: quarter.to_string(),
            cash_dividend,
            stock_dividend: Decimal::ZERO,
            ex_dividend_date1: ex_date.to_string(),
            ex_dividend_date2: "-".to_string(),
            payable_date1: "-".to_string(),
            payable_date2: "-".to_string(),
        }
    }

    /// 歷年明細全部寫入、季配會重算年度彙總列，且該股票會被列入季配/半年配股票清單。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn save_historical_dividends_writes_details_and_annual_total() {
        dotenvy::dotenv().ok();
        cleanup().await;

        let dividends = YahooDividend {
            stock_symbol: TEST_SYMBOL.to_string(),
            dividend: vec![(
                TEST_PAYOUT_YEAR,
                vec![
                    quarter_detail("Q2", dec!(1.5), "2020-09-15"),
                    quarter_detail("Q1", dec!(1.0), "2020-06-15"),
                ],
            )],
        };

        let upserted = save_historical_dividends(TEST_SYMBOL, &dividends)
            .await
            .expect("寫入歷年股利失敗");
        let annual_cash: Decimal = sqlx::query_scalar(
            "SELECT cash_dividend FROM dividend WHERE security_code = $1 AND year = $2 AND quarter = ''",
        )
        .bind(TEST_SYMBOL)
        .bind(TEST_PAYOUT_YEAR)
        .fetch_one(database::get_connection())
        .await
        .expect("讀取年度彙總列失敗");
        let symbols = multiple_dividend_symbols(TEST_PAYOUT_YEAR)
            .await
            .expect("查詢季配/半年配股票失敗");

        cleanup().await;

        assert_eq!(upserted, 2);
        assert_eq!(annual_cash, dec!(2.5));
        assert!(symbols.contains(TEST_SYMBOL));
    }
}
