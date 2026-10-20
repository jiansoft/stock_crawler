//! # Yahoo 三大財務報表採集
//!
//! 把 Yahoo 的損益表、資產負債表、現金流量表寫進 `income_statement`、`balance_sheet`、
//! `cash_flow_statement`。
//!
//! ## 每檔抓哪些期別（5 次請求）
//!
//! | 報表 | 單季 | 年度 | 累計 |
//! |------|:----:|:----:|:----:|
//! | 損益表 | ✅ | ✅ | ❌ |
//! | 現金流量表 | ✅ | ✅ | ❌ |
//! | 資產負債表 | ✅ | ❌（即 Q4） | ❌ |
//!
//! 累計金額可由單季加總推得，不另外抓；年度 EPS 是官方值、不等於各季相加，所以年度要抓。
//!
//! ## 排程節流
//!
//! 全市場約 1,800 檔，每檔 5 次請求。[`execute`] 每輪最多處理
//! [`MAX_STOCKS_PER_RUN`] 檔，處理過的股票以 Redis 旗標略過 [`REFRESH_TTL_SECONDS`]，
//! 因此每天跑一次、約 5 天掃完一輪、每檔約每週重抓一次。Yahoo 回 404 的代號
//! （多為已下市）略過 30 天。
//!
//! ## 季報期優先
//!
//! 官方季報（`financial_statement`，03:00 排程寫入）在 [`PRIORITY_LOOKBACK_DAYS`] 天內新公布、
//! 但損益表還沒有那一季的股票排在最前面，而且不看每週旗標、改用 [`PRIORITY_TTL_SECONDS`]
//! 的優先旗標：截止日前後每天有上百檔公布，不必等到下一次輪到才有新一季的單季財報。
//! Yahoo 還沒更新時隔天會再試，直到損益表追上官方或超過回看天數。
//!
//! ## 寫入的原子性
//!
//! 單檔的 5 次請求與轉譯全部成功後才開始寫入；任一步驟失敗，該檔一筆都不寫，
//! 不會留下「損益表是新的、現金流量表是舊的」這種半套狀態。
//!
//! ## BigGo 備援
//!
//! Yahoo 失敗（HTTP 500、格式異常等，404 除外）時改從 BigGo 財經採集（[`backfill_from_biggo`]），
//! 以 `source = 'biggo'` 寫入、與 Yahoo 的資料列並存；同樣全部成功才寫入。BigGo 只提供與
//! Yahoo 逐期吻合的欄位（營收、營業利益、稅後淨利、EPS、資產負債總額、五項現金流量等），
//! 其餘欄位為 `NULL`。Yahoo 回 404 多半是已下市，不走備援。

mod fetch;

use std::{collections::HashSet, future::Future, time::Duration};

use self::fetch::{StockReports, fetch_biggo_reports, fetch_reports};

use anyhow::{Context, Result, bail};
use rand::RngExt;

use crate::{
    domain::{
        financial::repository::FinancialReportRepository,
        registry::{entity::Stock, repository::StockRepository},
    },
    infra::{
        crawler::yahoo,
        database::repository::{
            financial_report::PgFinancialReportRepository, stock::PgStockRepository,
        },
        nosql::redis::CLIENT,
    },
};

/// 每輪排程最多處理的股票檔數。
///
/// 每檔約 6～7 秒（5 次請求加間隔），400 檔約 45 分鐘。
pub const MAX_STOCKS_PER_RUN: usize = 400;

/// 處理過的股票在這段時間內不重抓（7 天）。
pub const REFRESH_TTL_SECONDS: usize = 60 * 60 * 24 * 7;

/// 優先股票（損益表落後官方季報）在這段時間內不重抓（20 小時）：每天跑一次，隔天一定會再試。
pub const PRIORITY_TTL_SECONDS: usize = 60 * 60 * 20;

/// 官方季報在這段天數內新公布才列為優先，避免 Yahoo 一直沒有的代號天天重打。
pub const PRIORITY_LOOKBACK_DAYS: i32 = 30;

/// 上市、上櫃的 `stock_exchange_market_id`。
const LISTED_MARKET_IDS: [i32; 2] = [2, 4];

/// 單檔的寫入筆數。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SavedCounts {
    /// 損益表列數（單季＋年度）。
    pub income_statements: usize,
    /// 資產負債表列數（單季）。
    pub balance_sheets: usize,
    /// 現金流量表列數（單季＋年度）。
    pub cash_flow_statements: usize,
}

impl SavedCounts {
    /// 三張表合計列數。
    pub fn total(&self) -> usize {
        self.income_statements + self.balance_sheets + self.cash_flow_statements
    }
}

/// 一輪排程的執行結果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunSummary {
    /// 候選股票檔數。
    pub candidates: usize,
    /// 因 Redis 旗標而略過的檔數。
    pub skipped: usize,
    /// 實際嘗試的優先股票（損益表落後官方季報）檔數，已含在各結果的計數裡。
    pub prioritized: usize,
    /// 成功寫入的檔數（含沒有財報、寫入 0 列的標的）。
    pub succeeded: usize,
    /// Yahoo 失敗、改由 BigGo 備援寫入的檔數。
    pub fallback: usize,
    /// Yahoo 回 404 的檔數。
    pub not_found: usize,
    /// 失敗的檔數。
    pub failed: usize,
    /// 三張表合計寫入列數。
    pub rows: usize,
}

impl RunSummary {
    /// 本輪實際發出請求的檔數。
    pub fn attempted(&self) -> usize {
        self.succeeded + self.fallback + self.not_found + self.failed
    }
}

/// 排程入口：採集尚未在 [`REFRESH_TTL_SECONDS`] 內處理過的上市櫃股票。
///
/// 單檔失敗只記錄錯誤、不中斷整批。只有「有嘗試、卻全部失敗」時回傳錯誤，
/// 那通常代表 Yahoo 改版或被封鎖，需要人工介入。
pub async fn execute() -> Result<()> {
    let summary = run(MAX_STOCKS_PER_RUN).await?;

    tracing::info!(
        "Yahoo 財報採集結束: candidates={}, skipped={}, prioritized={}, succeeded={}, fallback={}, not_found={}, failed={}, rows={}",
        summary.candidates,
        summary.skipped,
        summary.prioritized,
        summary.succeeded,
        summary.fallback,
        summary.not_found,
        summary.failed,
        summary.rows
    );

    if summary.failed > 0 && summary.succeeded == 0 && summary.fallback == 0 {
        bail!(
            "Yahoo financial report backfill failed for all {} attempted stocks",
            summary.failed
        );
    }

    Ok(())
}

/// 一次採集全部尚未處理過的候選股票（不受 [`MAX_STOCKS_PER_RUN`] 限制）。
///
/// 供首次建檔一次補齊使用，全市場約需 3～4 小時。仍會讀寫 Redis 旗標，
/// 中斷後重跑會從未處理的股票接續。
pub async fn backfill_all() -> Result<RunSummary> {
    run(usize::MAX).await
}

/// 採集至多 `max_stocks` 檔尚未處理過的候選股票。
async fn run(max_stocks: usize) -> Result<RunSummary> {
    let stocks = PgStockRepository::new()
        .fetch_all_active()
        .await
        .context("fetch active stocks for financial report backfill failed")?;
    let repo = PgFinancialReportRepository::new();
    let (candidates, priority) = order_candidates(&repo, &stocks).await;
    run_candidates(
        &candidates,
        priority,
        max_stocks,
        &repo,
        fetch_reports,
        fetch_biggo_reports,
        stock_interval,
    )
    .await
}

/// 從股票主檔挑出候選股票並排出採集順序：損益表落後官方季報的在前，其餘依代號。
///
/// 回傳順序與優先檔數；查不到落後名單只是少了優先順序，照一般輪替採集即可。
async fn order_candidates(
    repo: &impl FinancialReportRepository,
    stocks: &[Stock],
) -> (Vec<String>, usize) {
    let mut candidates: Vec<String> = stocks
        .iter()
        .filter(|stock| is_candidate(stock))
        .map(|stock| stock.symbol().0.clone())
        .collect();
    candidates.sort();

    let lagging = repo
        .fetch_lagging_symbols(PRIORITY_LOOKBACK_DAYS)
        .await
        .unwrap_or_else(|why| {
            tracing::warn!("fetch lagging financial report symbols failed: {why:#}");
            Vec::new()
        });
    let (candidates, priority) = prioritize(candidates, &lagging);
    tracing::info!(
        "Yahoo 財報採集開始: candidates={}, priority={priority}",
        candidates.len()
    );
    (candidates, priority)
}

/// 把損益表落後官方季報的候選股票移到最前面（各自維持原順序），回傳新順序與優先檔數。
///
/// 不在候選名單裡的落後代號（如興櫃、ETF）不採集。
fn prioritize(candidates: Vec<String>, lagging: &[String]) -> (Vec<String>, usize) {
    let lagging: HashSet<&str> = lagging.iter().map(String::as_str).collect();
    let (mut ordered, rest): (Vec<String>, Vec<String>) = candidates
        .into_iter()
        .partition(|symbol| lagging.contains(symbol.as_str()));
    let priority = ordered.len();
    ordered.extend(rest);
    (ordered, priority)
}

/// 依序採集候選股票，跳過 Redis 旗標內處理過的，至多實際嘗試 `max_stocks` 檔。
///
/// 前 `priority` 檔是優先股票：只看 [`PRIORITY_TTL_SECONDS`] 的優先旗標、不看每週旗標，
/// 嘗試時兩個旗標都寫。Yahoo 失敗（404 除外）時改從 BigGo 採集。`yahoo`、`biggo` 是
/// 單檔報表的抓取函式，`pause` 是股票之間的間隔；測試時換成假資料與不等待。
async fn run_candidates<'a, Y, YF, B, BF, P, PF>(
    candidates: &'a [String],
    priority: usize,
    max_stocks: usize,
    repo: &impl FinancialReportRepository,
    yahoo: Y,
    biggo: B,
    pause: P,
) -> Result<RunSummary>
where
    Y: Fn(&'a str) -> YF,
    YF: Future<Output = Result<StockReports>>,
    B: Fn(&'a str) -> BF,
    BF: Future<Output = Result<StockReports>>,
    P: Fn() -> PF,
    PF: Future<Output = ()>,
{
    let mut summary = RunSummary {
        candidates: candidates.len(),
        ..Default::default()
    };

    for (index, stock_symbol) in candidates.iter().enumerate() {
        if summary.attempted() >= max_stocks {
            break;
        }

        let prioritized = index < priority;
        let cache_key = make_cache_key(stock_symbol);
        let skip_key = if prioritized {
            make_priority_cache_key(stock_symbol)
        } else {
            cache_key.clone()
        };
        if CLIENT
            .get_bool(&skip_key)
            .await
            .with_context(|| format!("redis get_bool failed: cache_key={skip_key}"))?
        {
            summary.skipped += 1;
            continue;
        }

        // 先寫旗標：即使這檔失敗，下一輪也不會立刻重打同一來源。
        if prioritized {
            summary.prioritized += 1;
            CLIENT
                .set(&skip_key, true, PRIORITY_TTL_SECONDS)
                .await
                .with_context(|| format!("redis set failed: cache_key={skip_key}"))?;
        }
        CLIENT
            .set(&cache_key, true, REFRESH_TTL_SECONDS)
            .await
            .with_context(|| format!("redis set failed: cache_key={cache_key}"))?;

        let yahoo_result = match yahoo(stock_symbol).await {
            Ok(reports) => save_reports(repo, stock_symbol, &reports).await,
            Err(why) => Err(why),
        };
        match yahoo_result {
            Ok(counts) => {
                summary.succeeded += 1;
                summary.rows += counts.total();
            }
            Err(why) if yahoo::dividend::is_page_not_found_error(&why) => {
                summary.not_found += 1;
                // 404 幾乎都是已下市、主檔旗標還沒更新的代號，拉長略過時間（優先旗標也一樣）。
                let keys = [Some(&cache_key), prioritized.then_some(&skip_key)];
                for key in keys.into_iter().flatten() {
                    if let Err(cache_err) = CLIENT
                        .set(key, true, yahoo::dividend::PAGE_NOT_FOUND_CACHE_TTL_SECONDS)
                        .await
                    {
                        tracing::error!(
                            "Failed to extend yahoo financial report 404 skip cache: stock_symbol={stock_symbol}, error={cache_err:#}"
                        );
                    }
                }
                tracing::warn!(
                    "skip financial report backfill because yahoo page not found (證券可能已下市): stock_symbol={stock_symbol}, error={why:#}"
                );
            }
            Err(why) => {
                pause().await;
                let fallback_result = match biggo(stock_symbol).await {
                    Ok(reports) => save_biggo_reports(repo, stock_symbol, &reports).await,
                    Err(fallback_err) => Err(fallback_err),
                };
                match fallback_result {
                    Ok(counts) => {
                        summary.fallback += 1;
                        summary.rows += counts.total();
                        tracing::warn!(
                            "financial report fell back to BigGo: stock_symbol={stock_symbol}, rows={}, yahoo_error={why:#}",
                            counts.total()
                        );
                    }
                    Err(fallback_err) => {
                        summary.failed += 1;
                        tracing::error!(
                            "financial report backfill failed: stock_symbol={stock_symbol}, error={why:#}, biggo_error={fallback_err:#}"
                        );
                    }
                }
            }
        }

        pause().await;
    }

    Ok(summary)
}

/// 採集單一股票的三大報表並寫入，不讀寫 Redis 旗標；手動回補也走這裡。
///
/// 沒有財報的標的（如 ETF）回傳全 0 的 [`SavedCounts`]。
///
/// # Errors
///
/// 任一次請求、解析、轉譯（如金額無法整除 1000）或寫入失敗時回傳錯誤。
/// 請求與轉譯階段失敗時不會寫入任何資料。
pub async fn backfill_for_stock(
    repo: &impl FinancialReportRepository,
    stock_symbol: &str,
) -> Result<SavedCounts> {
    let reports = fetch_reports(stock_symbol).await?;
    save_reports(repo, stock_symbol, &reports).await
}

/// 從 BigGo 採集單一股票的三大報表並寫入（`source = 'biggo'`），不讀寫 Redis 旗標。
///
/// Yahoo 失敗時的備援，手動回補也走這裡。BigGo 的年度資料含當年度累計，只寫入
/// 已有第四季單季資料的年度。
///
/// # Errors
///
/// 任一次請求、解析或轉譯失敗，或 BigGo 完全沒有這檔的財報時回傳錯誤；
/// 請求與轉譯階段失敗時不會寫入任何資料。
pub async fn backfill_from_biggo(
    repo: &impl FinancialReportRepository,
    stock_symbol: &str,
) -> Result<SavedCounts> {
    let reports = fetch_biggo_reports(stock_symbol).await?;
    save_biggo_reports(repo, stock_symbol, &reports).await
}

/// 寫入 BigGo 的單檔報表；BigGo 完全沒有這檔的財報視為失敗。
async fn save_biggo_reports(
    repo: &impl FinancialReportRepository,
    stock_symbol: &str,
    reports: &StockReports,
) -> Result<SavedCounts> {
    if reports.is_empty() {
        bail!("BigGo has no financial statements for {stock_symbol}");
    }
    save_reports(repo, stock_symbol, reports).await
}

/// 寫入單檔三大報表並回傳各表列數。
async fn save_reports(
    repo: &impl FinancialReportRepository,
    stock_symbol: &str,
    reports: &StockReports,
) -> Result<SavedCounts> {
    let income_statements = repo
        .save_income_statements(&reports.income_statements)
        .await
        .with_context(|| format!("save income statements failed: {stock_symbol}"))?;
    let balance_sheets = repo
        .save_balance_sheets(&reports.balance_sheets)
        .await
        .with_context(|| format!("save balance sheets failed: {stock_symbol}"))?;
    let cash_flow_statements = repo
        .save_cash_flow_statements(&reports.cash_flow_statements)
        .await
        .with_context(|| format!("save cash flow statements failed: {stock_symbol}"))?;

    tracing::debug!(
        "financial report saved: stock_symbol={stock_symbol}, income={income_statements}, balance={balance_sheets}, cash_flow={cash_flow_statements}"
    );

    Ok(SavedCounts {
        income_statements: income_statements as usize,
        balance_sheets: balance_sheets as usize,
        cash_flow_statements: cash_flow_statements as usize,
    })
}

/// 是否納入採集：上市櫃、非 ETF／ETN（`00` 開頭）、非特別股與受益證券（代號含英文字母）。
fn is_candidate(stock: &Stock) -> bool {
    let symbol = stock.symbol();
    LISTED_MARKET_IDS.contains(&stock.market_id())
        && !symbol.0.starts_with("00")
        && !symbol.is_preference()
}

/// Redis 略過旗標的 key。
fn make_cache_key(stock_symbol: &str) -> String {
    format!("yahoo:financial_report:{stock_symbol}")
}

/// 優先股票（損益表落後官方季報）的 Redis 略過旗標 key。
fn make_priority_cache_key(stock_symbol: &str) -> String {
    format!("yahoo:financial_report:priority:{stock_symbol}")
}

/// 股票之間隨機停 1.5～3 秒，降低規律請求被 Yahoo WAF 判定為爬蟲的機率。
async fn stock_interval() {
    let ms = rand::rng().random_range(1500..=3000);
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[cfg(test)]
mod tests;
