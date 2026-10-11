use std::sync::Mutex;

use anyhow::anyhow;
use async_trait::async_trait;

use super::*;
use crate::{
    core::declare::Quarter,
    domain::financial::statement::{
        BalanceSheet, CashFlowStatement, IncomeStatement, StatementPeriod,
    },
    infra::crawler::yahoo::dividend::YahooPageNotFoundError,
};

/// 記下寫入列數、回傳實際列數的假倉儲。
#[derive(Default)]
struct FakeRepository {
    saved: Mutex<Vec<(String, usize)>>,
}

#[async_trait]
impl FinancialReportRepository for FakeRepository {
    async fn save_income_statements(&self, statements: &[IncomeStatement]) -> Result<u64> {
        self.record("income", statements.len())
    }

    async fn save_balance_sheets(&self, sheets: &[BalanceSheet]) -> Result<u64> {
        self.record("balance", sheets.len())
    }

    async fn save_cash_flow_statements(&self, statements: &[CashFlowStatement]) -> Result<u64> {
        self.record("cash_flow", statements.len())
    }
}

impl FakeRepository {
    fn record(&self, table: &str, rows: usize) -> Result<u64> {
        self.saved.lock().unwrap().push((table.to_owned(), rows));
        Ok(rows as u64)
    }
}

fn income(stock_symbol: &str) -> IncomeStatement {
    IncomeStatement {
        stock_symbol: stock_symbol.to_owned(),
        fiscal_year: 2020,
        period: StatementPeriod::Single(Quarter::Q1),
        source: "yahoo".to_owned(),
        revenue: Some(1_000),
        gross_profit: None,
        selling_expenses: None,
        admin_expenses: None,
        rd_expenses: None,
        operating_expenses: None,
        operating_profit: None,
        non_operating_income: None,
        profit_before_tax: None,
        net_income: None,
        owner_parent_profit: None,
        revenue_per_share: None,
        operating_profit_per_share: None,
        profit_before_tax_per_share: None,
        eps: None,
        bps: None,
    }
}

/// 只有損益表一列的報表。
fn reports(stock_symbol: &str) -> StockReports {
    StockReports {
        income_statements: vec![income(stock_symbol)],
        balance_sheets: Vec::new(),
        cash_flow_statements: Vec::new(),
    }
}

fn empty_reports() -> StockReports {
    StockReports {
        income_statements: Vec::new(),
        balance_sheets: Vec::new(),
        cash_flow_statements: Vec::new(),
    }
}

/// 三張表依序寫入並回傳各表列數；BigGo 沒有任何報表時視為失敗、不寫入。
#[tokio::test]
async fn save_reports_writes_three_tables_and_rejects_empty_biggo() {
    let repo = FakeRepository::default();
    let counts = save_reports(&repo, "2330", &reports("2330"))
        .await
        .expect("save_reports");
    assert_eq!(
        counts,
        SavedCounts {
            income_statements: 1,
            balance_sheets: 0,
            cash_flow_statements: 0,
        }
    );
    let tables: Vec<String> = repo
        .saved
        .lock()
        .unwrap()
        .iter()
        .map(|(t, _)| t.clone())
        .collect();
    assert_eq!(tables, ["income", "balance", "cash_flow"]);

    let repo = FakeRepository::default();
    let err = save_biggo_reports(&repo, "2330", &empty_reports())
        .await
        .expect_err("BigGo 沒有報表應失敗");
    assert!(
        err.to_string()
            .contains("BigGo has no financial statements")
    );
    assert!(repo.saved.lock().unwrap().is_empty());
    assert!(empty_reports().is_empty() && !reports("2330").is_empty());
}

/// 假代號：F1 已有旗標、F2 Yahoo 成功、F3 Yahoo 404、F4 改用 BigGo、F5 兩邊都失敗、
/// F6 超過單輪上限不處理。
const SYMBOLS: [&str; 6] = [
    "79979F1", "79979F2", "79979F3", "79979F4", "79979F5", "79979F6",
];

async fn clear_flags() {
    for symbol in SYMBOLS {
        let _ = CLIENT.delete(&make_cache_key(symbol)).await;
    }
}

/// 旗標略過、Yahoo 成功、404、BigGo 備援、兩邊都失敗各自計數，實際嘗試數到上限就停。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn run_candidates_counts_each_outcome_and_stops_at_the_limit() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 run_candidates_counts_each_outcome_and_stops_at_the_limit：無 Redis 連線");
        return;
    }
    clear_flags().await;
    CLIENT
        .set(&make_cache_key("79979F1"), true, 60)
        .await
        .expect("預先寫入略過旗標");

    let candidates: Vec<String> = SYMBOLS.iter().map(|s| (*s).to_owned()).collect();
    let yahoo_calls = Mutex::new(Vec::new());
    let biggo_calls = Mutex::new(Vec::new());
    let repo = FakeRepository::default();
    let summary = run_candidates(
        &candidates,
        4,
        &repo,
        |symbol: &str| {
            yahoo_calls.lock().unwrap().push(symbol.to_owned());
            let result = match symbol {
                "79979F2" => Ok(reports(symbol)),
                "79979F3" => Err(anyhow::Error::new(YahooPageNotFoundError {
                    stock_symbol: symbol.to_owned(),
                    url: "https://example.invalid".to_owned(),
                })),
                _ => Err(anyhow!("Yahoo 測試失敗")),
            };
            async move { result }
        },
        |symbol: &str| {
            biggo_calls.lock().unwrap().push(symbol.to_owned());
            let result = match symbol {
                "79979F4" => Ok(reports(symbol)),
                _ => Ok(empty_reports()),
            };
            async move { result }
        },
        || async {},
    )
    .await
    .expect("run_candidates");

    assert_eq!(
        summary,
        RunSummary {
            candidates: 6,
            skipped: 1,
            succeeded: 1,
            fallback: 1,
            not_found: 1,
            failed: 1,
            rows: 2,
        }
    );
    assert_eq!(
        *yahoo_calls.lock().unwrap(),
        ["79979F2", "79979F3", "79979F4", "79979F5"]
    );
    assert_eq!(*biggo_calls.lock().unwrap(), ["79979F4", "79979F5"]);
    // 實際嘗試過的都寫了旗標；超過上限的 F6 沒有。
    for symbol in ["79979F2", "79979F3", "79979F4", "79979F5"] {
        assert!(
            CLIENT
                .get_bool(&make_cache_key(symbol))
                .await
                .unwrap_or(false),
            "{symbol}"
        );
    }
    assert!(
        !CLIENT
            .get_bool(&make_cache_key("79979F6"))
            .await
            .unwrap_or(false)
    );

    clear_flags().await;
}

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
        fallback: 1,
        not_found: 1,
        failed: 2,
        skipped: 10,
        ..Default::default()
    };
    assert_eq!(summary.attempted(), 7, "備援成功的檔數也算已嘗試");

    let counts = SavedCounts {
        income_statements: 2,
        balance_sheets: 3,
        cash_flow_statements: 4,
    };
    assert_eq!(counts.total(), 9);
}
