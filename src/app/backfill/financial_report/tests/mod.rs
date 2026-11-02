use std::sync::Mutex;

use anyhow::anyhow;
use async_trait::async_trait;

use super::*;
use crate::{
    core::declare::Quarter,
    domain::financial::statement::{
        BalanceSheet, CashFlowStatement, IncomeStatement, LaggingStatement, StatementPeriod,
    },
    infra::{crawler::yahoo::dividend::YahooPageNotFoundError, nosql::redis::CLIENT},
};

/// 記下寫入列數、回傳實際列數的假倉儲；`lagging` 為 `None` 時查落後名單失敗。
#[derive(Default)]
struct FakeRepository {
    saved: Mutex<Vec<(String, usize)>>,
    lagging: Option<Vec<String>>,
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

    async fn fetch_lagging_statements(
        &self,
        published_within_days: i32,
        min_age_days: i32,
    ) -> Result<Vec<LaggingStatement>> {
        assert_eq!(
            (published_within_days, min_age_days),
            (PRIORITY_LOOKBACK_DAYS, 0)
        );
        let symbols = self
            .lagging
            .clone()
            .ok_or_else(|| anyhow!("落後名單測試失敗"))?;
        Ok(symbols
            .into_iter()
            .map(|stock_symbol| LaggingStatement {
                stock_symbol,
                fiscal_year: 2026,
                quarter: "Q3".to_owned(),
                published_on: chrono::NaiveDate::from_ymd_opt(2026, 11, 10).expect("date"),
            })
            .collect())
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

/// 落後的候選股票移到最前面、各自維持原順序；不在候選名單的落後代號忽略。
#[test]
fn test_prioritize() {
    let candidates: Vec<String> = ["1101", "2330", "2454", "3008"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let lagging: Vec<String> = ["3008", "0050", "2330"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let (ordered, priority) = prioritize(candidates.clone(), &lagging);
    assert_eq!(ordered, ["2330", "3008", "1101", "2454"]);
    assert_eq!(priority, 2);

    assert_eq!(prioritize(candidates.clone(), &[]), (candidates, 0));
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
    assert_eq!(
        make_priority_cache_key("2330"),
        "yahoo:financial_report:priority:2330"
    );
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

mod run_candidates;
