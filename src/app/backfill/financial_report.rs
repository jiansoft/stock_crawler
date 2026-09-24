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
//! ## 寫入的原子性
//!
//! 單檔的 5 次請求與轉譯全部成功後才開始寫入；任一步驟失敗，該檔一筆都不寫，
//! 不會留下「損益表是新的、現金流量表是舊的」這種半套狀態。

use std::time::Duration;

use anyhow::{Context, Result, bail};
use rand::RngExt;

use crate::{
    app::backfill::acl::YahooFinancialReportAclMapper,
    domain::{
        financial::{
            repository::FinancialReportRepository,
            statement::{BalanceSheet, CashFlowStatement, IncomeStatement},
        },
        registry::{entity::Stock, repository::StockRepository},
    },
    infra::{
        crawler::yahoo::{
            self,
            financial_statement::{ReportPeriod, balance_sheet, cash_flow, income_statement},
        },
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
    /// 成功寫入的檔數（含沒有財報、寫入 0 列的標的）。
    pub succeeded: usize,
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
        self.succeeded + self.not_found + self.failed
    }
}

/// 排程入口：採集尚未在 [`REFRESH_TTL_SECONDS`] 內處理過的上市櫃股票。
///
/// 單檔失敗只記錄錯誤、不中斷整批。只有「有嘗試、卻全部失敗」時回傳錯誤，
/// 那通常代表 Yahoo 改版或被封鎖，需要人工介入。
pub async fn execute() -> Result<()> {
    let summary = run(MAX_STOCKS_PER_RUN).await?;

    tracing::info!(
        "Yahoo 財報採集結束: candidates={}, skipped={}, succeeded={}, not_found={}, failed={}, rows={}",
        summary.candidates,
        summary.skipped,
        summary.succeeded,
        summary.not_found,
        summary.failed,
        summary.rows
    );

    if summary.failed > 0 && summary.succeeded == 0 {
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
    let mut candidates: Vec<String> = stocks
        .iter()
        .filter(|stock| is_candidate(stock))
        .map(|stock| stock.symbol().0.clone())
        .collect();
    candidates.sort();

    let repo = PgFinancialReportRepository::new();
    let mut summary = RunSummary {
        candidates: candidates.len(),
        ..Default::default()
    };
    tracing::info!("Yahoo 財報採集開始: candidates={}", summary.candidates);

    for stock_symbol in &candidates {
        if summary.attempted() >= max_stocks {
            break;
        }

        let cache_key = make_cache_key(stock_symbol);
        if CLIENT
            .get_bool(&cache_key)
            .await
            .with_context(|| format!("redis get_bool failed: cache_key={cache_key}"))?
        {
            summary.skipped += 1;
            continue;
        }

        // 先寫旗標：即使這檔失敗，下一輪也不會立刻重打同一來源。
        CLIENT
            .set(&cache_key, true, REFRESH_TTL_SECONDS)
            .await
            .with_context(|| format!("redis set failed: cache_key={cache_key}"))?;

        match backfill_for_stock(&repo, stock_symbol).await {
            Ok(counts) => {
                summary.succeeded += 1;
                summary.rows += counts.total();
            }
            Err(why) if yahoo::dividend::is_page_not_found_error(&why) => {
                summary.not_found += 1;
                // 404 幾乎都是已下市、主檔旗標還沒更新的代號，拉長略過時間。
                if let Err(cache_err) = CLIENT
                    .set(
                        &cache_key,
                        true,
                        yahoo::dividend::PAGE_NOT_FOUND_CACHE_TTL_SECONDS,
                    )
                    .await
                {
                    tracing::error!(
                        "Failed to extend yahoo financial report 404 skip cache: stock_symbol={stock_symbol}, error={cache_err:#}"
                    );
                }
                tracing::warn!(
                    "skip financial report backfill because yahoo page not found (證券可能已下市): stock_symbol={stock_symbol}, error={why:#}"
                );
            }
            Err(why) => {
                summary.failed += 1;
                tracing::error!(
                    "financial report backfill failed: stock_symbol={stock_symbol}, error={why:#}"
                );
            }
        }

        stock_interval().await;
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

/// 單檔三大報表轉譯後的領域實體。
struct StockReports {
    income_statements: Vec<IncomeStatement>,
    balance_sheets: Vec<BalanceSheet>,
    cash_flow_statements: Vec<CashFlowStatement>,
}

/// 依序發出 5 次請求並轉譯；任一步失敗即回傳錯誤。
async fn fetch_reports(stock_symbol: &str) -> Result<StockReports> {
    let mut income_statements = Vec::new();
    let mut cash_flow_statements = Vec::new();

    for period in [ReportPeriod::Quarter, ReportPeriod::Year] {
        let rows = income_statement::visit(stock_symbol, period)
            .await
            .with_context(|| {
                format!("fetch income statements failed: {stock_symbol} {period:?}")
            })?;
        for row in &rows {
            income_statements.push(YahooFinancialReportAclMapper::income_statement(row)?);
        }
        request_interval().await;

        let rows = cash_flow::visit(stock_symbol, period)
            .await
            .with_context(|| {
                format!("fetch cash flow statements failed: {stock_symbol} {period:?}")
            })?;
        for row in &rows {
            cash_flow_statements.push(YahooFinancialReportAclMapper::cash_flow_statement(row)?);
        }
        request_interval().await;
    }

    let balance_sheets = balance_sheet::visit(stock_symbol)
        .await
        .with_context(|| format!("fetch balance sheets failed: {stock_symbol}"))?
        .iter()
        .map(YahooFinancialReportAclMapper::balance_sheet)
        .collect::<Result<Vec<_>>>()?;

    Ok(StockReports {
        income_statements,
        balance_sheets,
        cash_flow_statements,
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

/// 同一檔股票的請求之間隨機停 0.3～0.8 秒。
async fn request_interval() {
    let ms = rand::rng().random_range(300..=800);
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// 股票之間隨機停 1.5～3 秒，降低規律請求被 Yahoo WAF 判定為爬蟲的機率。
async fn stock_interval() {
    let ms = rand::rng().random_range(1500..=3000);
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stock(symbol: &str, market_id: i32) -> Stock {
        Stock::register(symbol.to_string(), "測試".to_string(), market_id, 1)
    }

    #[test]
    fn test_is_candidate() {
        assert!(is_candidate(&stock("2330", 2)));
        assert!(is_candidate(&stock("8042", 4)));

        // 興櫃、公開發行不採集。
        assert!(!is_candidate(&stock("7708", 5)));
        assert!(!is_candidate(&stock("1234", 1)));
        // ETF／ETN、槓桿反向 ETF、特別股、受益證券。
        assert!(!is_candidate(&stock("0050", 2)));
        assert!(!is_candidate(&stock("00679B", 4)));
        assert!(!is_candidate(&stock("2881A", 2)));
        assert!(!is_candidate(&stock("01001T", 2)));
    }

    #[test]
    fn test_make_cache_key() {
        assert_eq!(make_cache_key("2330"), "yahoo:financial_report:2330");
    }

    #[test]
    fn test_summary_counts() {
        let summary = RunSummary {
            succeeded: 3,
            not_found: 1,
            failed: 2,
            skipped: 10,
            ..Default::default()
        };
        assert_eq!(summary.attempted(), 6);

        let counts = SavedCounts {
            income_statements: 2,
            balance_sheets: 3,
            cash_flow_statements: 4,
        };
        assert_eq!(counts.total(), 9);
    }
}
