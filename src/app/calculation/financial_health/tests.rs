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

    async fn fetch_financial_health(&self) -> Result<Vec<FinancialHealthSnapshot>> {
        Ok(self.written.lock().expect("lock").clone())
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

    let rebuilt = rebuild(&repo, 2026).await.expect("rebuild");
    let summary = rebuilt.summary;
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
    let written = repo.fetch_financial_health().await.expect("fetch");
    assert_eq!(written, rebuilt.snapshots, "寫入的就是回傳的快照");
    assert_eq!(written.len(), 2);
    assert_eq!(written[1].f_score, None, "B 沒有年度損益表，不算 F 分數");
}

/// 年報、季報都讀不到時回錯誤，不呼叫整批取代，上一次的結果原封不動。
#[tokio::test]
async fn rebuild_keeps_previous_snapshots_when_nothing_is_read() {
    let previous = build_snapshots(
        vec![("A".to_owned(), 2025, annual(true, 150))],
        vec![("A".to_owned(), quarter(2026, 2))],
    );
    let repo = FakeRepository {
        written: Mutex::new(previous.clone()),
        ..FakeRepository::default()
    };

    let err = rebuild(&repo, 2026).await.expect_err("0 檔要回錯誤");
    assert!(err.to_string().contains("不清空"), "{err:#}");
    let written = repo.fetch_financial_health().await.expect("fetch");
    assert_eq!(written, previous, "沒有讀到任何財報時不能清掉舊結果");
}

/// 以臨時資料庫跑排程本體：假股票 79979FH 1998、1999 年報（營收、毛利、淨利），以 2000 年為基準
/// 重算後寫入財務體質；結束時刪除財報與財務體質。今天的年度沒有財報時入口回「0 檔」錯誤、不清空舊結果。
mod database {
    use super::*;
    use crate::infra::database;

    const SYMBOL: &str = "79979FH";

    async fn cleanup() {
        let pool = database::get_connection();
        for sql in [
            "DELETE FROM income_statement WHERE stock_symbol = $1",
            "DELETE FROM financial_health WHERE stock_symbol = $1",
        ] {
            sqlx::query(sql)
                .bind(SYMBOL)
                .execute(pool)
                .await
                .expect("清除測試資料");
        }
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn run_rebuilds_from_annual_statements() {
        dotenvy::dotenv().ok();
        let pool = database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 run_rebuilds_from_annual_statements：無資料庫連接");
            return;
        }
        cleanup().await;
        for (year, revenue, gross, net) in
            [(1998, 1_000_i64, 300_i64, 100_i64), (1999, 1_200, 400, 150)]
        {
            sqlx::query(
                "INSERT INTO income_statement (stock_symbol, fiscal_year, period_type, quarter, revenue, gross_profit, net_income) \
                 VALUES ($1, $2, 'annual', 'A', $3, $4, $5)",
            )
            .bind(SYMBOL)
            .bind(year)
            .bind(revenue)
            .bind(gross)
            .bind(net)
            .execute(pool)
            .await
            .expect("插入年報");
        }

        let fixed = run(2000).await;
        let rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM financial_health WHERE stock_symbol = $1")
                .bind(SYMBOL)
                .fetch_one(pool)
                .await
                .expect("讀回財務體質");
        cleanup().await;
        let today = execute().await;

        fixed.expect("以 2000 年為基準重算");
        assert_eq!(rows, 1, "寫入假股票的財務體質");
        assert!(
            today
                .as_ref()
                .map_or_else(|why| why.to_string().contains("0 檔"), |()| true),
            "今天的年度沒有財報時只能是 0 檔錯誤：{today:?}"
        );
    }
}
