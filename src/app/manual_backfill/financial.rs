//! 手動回補：三大財務報表與財務體質。
//!
//! - `test_backfill_financial_reports_for_symbols`：
//!   依 [`MANUAL_FINANCIAL_REPORT_SYMBOLS`] 從 Yahoo 採集指定股票的三大財務報表，
//!   不讀寫 Redis 略過旗標。
//! - `test_backfill_financial_reports_from_biggo_for_symbols`：
//!   依 [`MANUAL_FINANCIAL_REPORT_SYMBOLS`] 改從 BigGo 採集指定股票的三大財務報表
//!   （`source = 'biggo'`，Yahoo 長期失敗或沒有資料時補用）。
//! - `test_backfill_financial_reports_all`：
//!   一次採集全部上市櫃股票的三大財務報表（首次建檔用，約 3～4 小時）。
//! - `test_rebuild_financial_health`：
//!   立即以三大財務報表重算全市場財務體質，整批取代 `financial_health`
//!   （等同 07:10 排程；首次建表或補抓財報後不想等排程時使用）。

use crate::app::backfill::financial_report;

/// 手動採集三大財務報表的預設股票代號（逗號分隔）。
///
/// 可用環境變數 `MANUAL_FINANCIAL_REPORT_SYMBOLS` 覆蓋。
const MANUAL_FINANCIAL_REPORT_SYMBOLS: &str = "8042,2330,2881";

/// 採集指定股票的 Yahoo 三大財務報表（損益表、資產負債表、現金流量表）。
///
/// 每檔 5 次請求，不讀寫 Redis 略過旗標，適合補單檔或驗證資料。
/// 任一檔失敗即中止，方便看到錯誤原因。
///
/// `cargo test app::manual_backfill::financial::test_backfill_financial_reports_for_symbols -- --ignored --nocapture`
/// 可用 `MANUAL_FINANCIAL_REPORT_SYMBOLS=2330,2317` 指定股票。
#[tokio::test]
#[ignore]
async fn test_backfill_financial_reports_for_symbols() {
    dotenvy::dotenv().ok();

    let symbols = std::env::var("MANUAL_FINANCIAL_REPORT_SYMBOLS")
        .unwrap_or_else(|_| MANUAL_FINANCIAL_REPORT_SYMBOLS.to_string());
    let repo =
        crate::infra::database::repository::financial_report::PgFinancialReportRepository::new();

    for symbol in symbols.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let counts = financial_report::backfill_for_stock(&repo, symbol)
            .await
            .unwrap_or_else(|why| panic!("financial report backfill failed for {symbol}: {why:#}"));
        println!(
            "{symbol}: income={} balance={} cash_flow={}",
            counts.income_statements, counts.balance_sheets, counts.cash_flow_statements
        );
    }
}

/// 從 BigGo 採集指定股票的三大財務報表（Yahoo 的備援來源，`source = 'biggo'`）。
///
/// 每檔 9 次請求，不讀寫 Redis 略過旗標。任一檔失敗即中止，方便看到錯誤原因。
///
/// `cargo test app::manual_backfill::financial::test_backfill_financial_reports_from_biggo_for_symbols -- --ignored --nocapture`
/// 可用 `MANUAL_FINANCIAL_REPORT_SYMBOLS=2527,9938` 指定股票。
#[tokio::test]
#[ignore]
async fn test_backfill_financial_reports_from_biggo_for_symbols() {
    dotenvy::dotenv().ok();

    let symbols = std::env::var("MANUAL_FINANCIAL_REPORT_SYMBOLS")
        .unwrap_or_else(|_| MANUAL_FINANCIAL_REPORT_SYMBOLS.to_string());
    let repo =
        crate::infra::database::repository::financial_report::PgFinancialReportRepository::new();

    for symbol in symbols.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let counts = financial_report::backfill_from_biggo(&repo, symbol)
            .await
            .unwrap_or_else(|why| {
                panic!("BigGo financial report backfill failed for {symbol}: {why:#}")
            });
        println!(
            "{symbol}: income={} balance={} cash_flow={}",
            counts.income_statements, counts.balance_sheets, counts.cash_flow_statements
        );
    }
}

/// 一次採集全部上市櫃股票的 Yahoo 三大財務報表（首次建檔用）。
///
/// 約 1,800 檔、每檔 5 次請求，需要 3～4 小時。會寫入 7 天的 Redis 略過旗標，
/// 中斷後重跑會從未處理的股票接續；排程之後也會自然接手。
///
/// `cargo test app::manual_backfill::financial::test_backfill_financial_reports_all -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_financial_reports_all() {
    dotenvy::dotenv().ok();

    let summary = financial_report::backfill_all()
        .await
        .expect("financial report backfill failed");

    println!("結束 test_backfill_financial_reports_all {summary:?}");
}

/// 立即重算全市場財務體質，整批取代 `financial_health`。
///
/// 等同 07:10 排程 `calculation::financial_health::execute`，只讀三大財務報表、
/// 不呼叫外部來源。
///
/// 執行範例：
/// `cargo test app::manual_backfill::financial::test_rebuild_financial_health -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_rebuild_financial_health() {
    dotenvy::dotenv().ok();

    crate::app::calculation::financial_health::execute()
        .await
        .expect("manual financial health rebuild failed");
}
