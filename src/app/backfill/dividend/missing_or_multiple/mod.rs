mod plan;

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use rand::RngExt;

use self::plan::plan_stock_dividends;
use crate::{
    app::calculation::dividend_record, core::util::map::Keyable,
    domain::dividend::entity::Dividend, domain::dividend::repository::DividendRepository,
    infra::crawler::yahoo, infra::database::repository::dividend::PgDividendRepository,
};
use anyhow::{Context, Result};

/// 跳過旗標存活時間下限（2 天）。
///
/// 旗標原本固定 3 天，同一輪寫入的旗標會在同一天一起到期：2026-10-06 一次重抓上千檔跑了
/// 70 分鐘，隔兩天卻只抓幾十檔。改在下限與上限之間隨機，到期日會逐輪攤平到每天，
/// 平均仍約 3 天重抓一次，對 Yahoo 的總請求量不變。
const REFRESH_SKIP_TTL_MIN_SECONDS: usize = 60 * 60 * 24 * 2;
/// 跳過旗標存活時間上限（4 天），見 [`REFRESH_SKIP_TTL_MIN_SECONDS`]。
const REFRESH_SKIP_TTL_MAX_SECONDS: usize = 60 * 60 * 24 * 4;
/// 單輪最多實際查詢 Yahoo 的檔數。
///
/// 約 2,355 檔÷平均 3 天≈每天 785 檔，上限留一些餘裕；遇到大量旗標同時到期時，
/// 超出的股票沒有寫旗標，下一輪自然會再排到，避免單輪拖到 22:00 之後的排程。
const MAX_FETCHES_PER_RUN: usize = 900;

/// 單輪股利補抓的查詢額度與統計。
///
/// 已有跳過旗標的股票不經過這裡；需要查詢的股票逐一呼叫 [`Self::try_fetch`]，
/// 超過 [`MAX_FETCHES_PER_RUN`] 的記為延後。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RefreshRun {
    /// 本輪候選股票數。
    candidates: usize,
    /// 實際查詢 Yahoo 的股票數。
    fetched: usize,
    /// 額度用完而延到下一輪的股票數。
    deferred: usize,
}

impl RefreshRun {
    fn new(candidates: usize) -> Self {
        Self {
            candidates,
            fetched: 0,
            deferred: 0,
        }
    }

    /// 占用一次查詢額度；額度已用完時記為延後並回傳 `false`。
    fn try_fetch(&mut self) -> bool {
        if self.fetched >= MAX_FETCHES_PER_RUN {
            self.deferred += 1;
            return false;
        }
        self.fetched += 1;
        true
    }

    /// 因跳過旗標未到期而略過的股票數。
    fn skipped(&self) -> usize {
        self.candidates - self.fetched - self.deferred
    }
}

impl std::fmt::Display for RefreshRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "candidates={} fetched={} skipped={} deferred={}",
            self.candidates,
            self.fetched,
            self.skipped(),
            self.deferred
        )
    }
}

/// 定期檢查上市櫃股票的近期股利，補齊缺漏或新增的配息。
///
/// 此流程會先建立兩份資料：
/// 1. 待採集股票清單：包含仍在上市櫃的股票，以及已有季配/半年配紀錄的股票。
/// 2. 多次配息快取：用 `security_code-year_of_dividend-quarter` 記住既有季配/半年配資料。
///
/// 多次配息查詢會同時涵蓋 `year` 與 `year_of_dividend`，因為剛跨年度時常見
/// 「去年 Q4 股利於今年發放」的資料。後續抓 Yahoo 後只處理指定年度與前一年度的
/// 股利所屬年度，並依快取排除已存在的季配/半年配紀錄，再將缺少的資料 upsert 回資料庫。
///
/// Redis 會以股票代碼建立 2～4 天的隨機跳過旗標，避免排程短時間內重複打 Yahoo，
/// 每輪最多查 [`MAX_FETCHES_PER_RUN`] 檔。單檔股票失敗時只寫 log，不中斷整批採集。
pub(super) async fn backfill_missing_or_multiple_dividends(year: i32) -> Result<()> {
    let dividend_repo = PgDividendRepository::new();
    // 年配公司也可能新增半年配，不能用已有年度資料排除；重抓頻率交給 2～4 天的跳過旗標。
    let mut stock_symbols: HashSet<String> = dividend_repo
        .fetch_dividend_refresh_candidates()
        .await?
        .into_iter()
        .collect();
    // 再找出與指定年度相關的季配/半年配資料；查詢同時看發放年度與股利所屬年度，避免跨年度漏判。
    let multiple_dividends = dividend_repo
        .fetch_multiple_dividends_for_year(year)
        .await?;
    let mut multiple_dividend_cache: HashMap<String, Dividend> = HashMap::new();
    for dividend in multiple_dividends {
        let key = dividend.key();
        // 已有季配/半年配的股票也要重新掃描，才能補齊缺漏季度或重算年度彙總列。
        stock_symbols.insert(dividend.security_code.to_string());
        // 快取 key 與 Yahoo 回補時的 key 相同；連同實體一起留著，才能判斷來源是否真的有異動。
        multiple_dividend_cache.insert(key, dividend);
    }

    tracing::info!("本次殖利率的採集需收集 {} 家", stock_symbols.len());
    let mut run = RefreshRun::new(stock_symbols.len());
    for stock_symbol in stock_symbols {
        // Yahoo 股利回補使用獨立快取命名空間，避免和 Goodinfo 盈餘分配率快取互相影響。
        let cache_key = make_cache_key(&stock_symbol);
        let is_jump = crate::infra::nosql::redis::CLIENT
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

        if !run.try_fetch() {
            // 本輪額度已用完；不寫旗標，下一輪會再排到這檔。
            continue;
        }

        // 先寫入跳過旗標；即使單檔處理失敗，也避免排程下一輪立刻重試同一來源。
        let ttl_seconds = refresh_skip_ttl_seconds();
        crate::infra::nosql::redis::CLIENT
            .set(cache_key, true, ttl_seconds)
            .await
            .with_context(|| {
                format!(
                    "redis set failed: year={}, stock_symbol={}, ttl_seconds={}",
                    year, stock_symbol, ttl_seconds
                )
            })?;

        // 單檔股票的抓取、轉換或入庫失敗只記錄錯誤，整批任務繼續處理其他股票。
        if let Err(why) =
            backfill_recent_dividends_for_stock(year, &stock_symbol, &multiple_dividend_cache).await
        {
            if yahoo::dividend::is_page_not_found_error(&why) {
                // Yahoo 回 404＝整個個股頁不存在，幾乎都是已下市/清算的標的
                //（資料庫的 SuspendListing 旗標可能還沒更新）。這不是異常，
                // 降級為 warn 避免淹沒真正的錯誤，並把跳過快取拉長到 30 天，
                // 不再每 2～4 天重打一次注定 404 的請求。
                if let Err(cache_err) = crate::infra::nosql::redis::CLIENT
                    .set(
                        make_cache_key(&stock_symbol),
                        true,
                        yahoo::dividend::PAGE_NOT_FOUND_CACHE_TTL_SECONDS,
                    )
                    .await
                {
                    tracing::error!(
                        "Failed to extend yahoo dividend 404 skip cache: stock_symbol={}, error={:#}",
                        stock_symbol,
                        cache_err
                    );
                }
                tracing::warn!(
                    "skip dividend backfill because yahoo page not found (證券可能已下市): year={}, stock_symbol={}, error={:#}",
                    year,
                    stock_symbol,
                    why
                );
            } else {
                tracing::error!(
                    "backfill_missing_or_multiple_dividends failed: year={}, stock_symbol={}, stage=backfill_recent_dividends_for_stock, error={:#}",
                    year,
                    stock_symbol,
                    why
                );
            }
        }

        // 每檔股票請求完成後，進行隨機 1.5 到 3.0 秒的延遲（Jitter），降低規律請求被 Yahoo WAF 偵測為爬蟲的機率
        let jitter_ms = rand::rng().random_range(1500..=3000);
        tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
    }

    tracing::info!("殖利率採集完成 {run}");

    Ok(())
}

/// 處理單一股票的近期股利抓取、去重、入庫與年度彙總更新。
///
/// 主要步驟：
/// 1. 從 Yahoo 取得該股票依發放年度分組的股利明細。
/// 2. 僅保留指定年度與前一年度的股利所屬年度，避免回寫太舊的歷史資料。
/// 3. 對季配/半年配資料使用 `multiple_dividend_cache` 排除已存在紀錄。
/// 4. 將缺少的 Yahoo 明細轉為 `Dividend` 後 upsert。
/// 5. 如果新增或更新分期明細（含混合配息的全年 A），重算年度合計與持股領取紀錄。
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
    multiple_dividend_cache: &HashMap<String, Dividend>,
) -> Result<()> {
    let dividend_repo = PgDividendRepository::new();
    // 先從 Yahoo 讀取單一股票的股利頁面；這一步失敗代表該股票無法繼續處理，所以直接向外回錯。
    let dividends_from_yahoo = yahoo::dividend::visit(stock_symbol)
        .await
        .with_context(|| {
            format!(
                "yahoo dividend fetch failed: year={}, stock_symbol={}",
                year, stock_symbol
            )
        })?;
    let plan = plan_stock_dividends(
        year,
        stock_symbol,
        &dividends_from_yahoo,
        multiple_dividend_cache,
    );
    // 是否真的寫入了季配/半年配明細；只有這種情況才需要連帶回補持股的領息紀錄。
    let mut saved_multiple_dividends = false;
    for entity in &plan.to_save {
        match dividend_repo.save(entity).await {
            Ok(_) => {
                tracing::debug!("dividend upsert executed successfully. \r\n{:#?}", entity);

                if !entity.quarter.is_empty() {
                    saved_multiple_dividends = true;
                }
            }
            Err(why) => {
                // 單筆 upsert 失敗只記錄錯誤，讓同檔股票其他季度仍有機會完成入庫。
                tracing::error!(
                    "dividend upsert failed: year={}, paid_year={}, stock_symbol={}, year_of_dividend={}, quarter={}, error={:#}",
                    year,
                    entity.year,
                    stock_symbol,
                    entity.year_of_dividend,
                    entity.quarter,
                    why
                );
            }
        }
    }

    for refresh_year in plan.refresh_years {
        if let Err(why) = dividend_repo
            .upsert_annual_total_dividend(stock_symbol, refresh_year)
            .await
        {
            // 年度彙總失敗不影響已寫入的季配/半年配明細，因此記錄後繼續處理下一個年度。
            tracing::error!(
                "upsert_annual_total_dividend failed: year={}, stock_symbol={}, refresh_year={}, error={:#}",
                year,
                stock_symbol,
                refresh_year,
                why
            );
        }
    }

    if saved_multiple_dividends {
        // 新發現的半年配可能早已除息，必須同步補持股紀錄，前端才查得到這次配息。
        dividend_record::backfill_received_dividend_records_for_stock(stock_symbol).await?;
    }

    Ok(())
}

/// 建立 Yahoo 股利回補用的 Redis 快取 key。
///
/// 快取只以股票代號為粒度，目的是限制短時間內對同一 Yahoo 股利頁面的重複請求。
/// key 前綴固定為 `yahoo:dividend:`，避免與 Goodinfo 或其他資料來源的快取混用。
fn make_cache_key(stock_symbol: &str) -> String {
    format!("yahoo:dividend:{stock_symbol}")
}

/// 隨機挑一個跳過旗標存活秒數，範圍見 [`REFRESH_SKIP_TTL_MIN_SECONDS`]。
fn refresh_skip_ttl_seconds() -> usize {
    rand::rng().random_range(REFRESH_SKIP_TTL_MIN_SECONDS..=REFRESH_SKIP_TTL_MAX_SECONDS)
}

#[cfg(test)]
mod tests;
