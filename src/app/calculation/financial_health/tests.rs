//! 財務體質每日重算：以假倉儲驗證分組、讀取年度範圍與統計。

use std::sync::Mutex;

use async_trait::async_trait;

use super::*;
use crate::domain::financial::health::YearFigures;

/// 回傳固定數值、記下讀取年度與寫入內容的假倉儲。
#[derive(Default)]
struct FakeRepository {
    annual: Vec<(String, i32, AnnualFigures)>,
    quarters: Vec<(String, QuarterFigures)>,
    since_years: Mutex<Vec<i32>>,
    written: Mutex<Vec<FinancialHealthSnapshot>>,
}

#[async_trait]
impl FinancialHealthRepository for FakeRepository {
    async fn fetch_annual_figures(
        &self,
        since_year: i32,
    ) -> Result<Vec<(String, i32, AnnualFigures)>> {
        self.since_years.lock().expect("lock").push(since_year);
        Ok(self.annual.clone())
    }

    async fn fetch_quarter_figures(
        &self,
        since_year: i32,
    ) -> Result<Vec<(String, QuarterFigures)>> {
        self.since_years.lock().expect("lock").push(since_year);
        Ok(self.quarters.clone())
    }

    async fn replace_financial_health(&self, snapshots: &[FinancialHealthSnapshot]) -> Result<u64> {
        *self.written.lock().expect("lock") = snapshots.to_vec();
        Ok(snapshots.len() as u64)
    }
}

fn annual(has_income: bool, net_income: i64) -> AnnualFigures {
    AnnualFigures {
        has_income,
        figures: YearFigures {
            revenue: Some(1_000),
            gross_profit: Some(300),
            net_income: Some(net_income),
            operating_cash_flow: Some(200),
            total_assets: Some(10_000),
            current_assets: Some(4_000),
            current_liabilities: Some(2_000),
            non_current_liabilities: Some(1_000),
            share_capital: Some(500),
        },
    }
}

fn quarter(fiscal_year: i32, quarter: u8) -> QuarterFigures {
    QuarterFigures {
        fiscal_year,
        quarter,
        has_income: true,
        revenue: Some(250),
        gross_profit: Some(100),
        total_assets: Some(10_000),
        total_liabilities: Some(4_000),
        free_cash_flow: Some(50),
    }
}

/// 年報與季報依股票分組：A 兩者都有、B 只有年報、C 只有季報；結果依代號排序。
#[test]
fn build_snapshots_groups_by_stock() {
    let snapshots = build_snapshots(
        vec![
            ("B".to_owned(), 2024, annual(true, 100)),
            ("A".to_owned(), 2024, annual(true, 100)),
            ("A".to_owned(), 2025, annual(true, 150)),
        ],
        vec![
            ("C".to_owned(), quarter(2026, 1)),
            ("A".to_owned(), quarter(2025, 4)),
            ("A".to_owned(), quarter(2026, 1)),
            ("A".to_owned(), quarter(2025, 3)),
            ("A".to_owned(), quarter(2025, 2)),
        ],
    );

    let symbols: Vec<_> = snapshots.iter().map(|s| s.stock_symbol.as_str()).collect();
    assert_eq!(symbols, ["A", "B", "C"]);
    let a = &snapshots[0];
    assert_eq!(a.f_score_year, Some(2025));
    assert_eq!((a.quarter_year, a.quarter), (Some(2026), Some(1)));
    assert_eq!(a.free_cash_flow_ttm, Some(200));
    let b = &snapshots[1];
    assert_eq!((b.f_score_year, b.quarter), (Some(2024), None));
    let c = &snapshots[2];
    assert_eq!((c.f_score, c.quarter_year), (None, Some(2026)));
}

/// 年報往回讀 3 年、季報 2 年；寫入全部快照並回傳統計。
#[tokio::test]
async fn rebuild_reads_lookback_years_and_replaces_all_snapshots() {
    let repo = FakeRepository {
        annual: vec![
            ("A".to_owned(), 2025, annual(true, 150)),
            ("B".to_owned(), 2025, annual(false, 0)),
        ],
        quarters: vec![("A".to_owned(), quarter(2026, 2))],
        ..FakeRepository::default()
    };

    let summary = rebuild(&repo, 2026).await.expect("rebuild");
    assert_eq!(
        summary,
        RebuildSummary {
            stocks: 2,
            scored: 1,
            with_quarter: 1,
            written: 2,
        }
    );
    assert_eq!(
        summary.to_string(),
        "stocks=2 scored=1 with_quarter=1 written=2"
    );
    assert_eq!(*repo.since_years.lock().expect("lock"), vec![2023, 2024]);
    let written = repo.written.lock().expect("lock");
    assert_eq!(written.len(), 2);
    assert_eq!(written[1].f_score, None, "B 沒有年度損益表，不算 F 分數");
}
