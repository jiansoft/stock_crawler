//! 財報落後（官方季報已公布、損益表還沒有那一季）的檢查。

use super::*;

fn lagging(symbol: &str, published: u32) -> LaggingStatement {
    LaggingStatement {
        stock_symbol: symbol.to_owned(),
        fiscal_year: 2026,
        quarter: "Q2".to_owned(),
        published_on: day(published),
    }
}

/// 財報落後：沒有落後不列、列出代號與期別與公布日、超過上限只計數；查詢失敗也是問題。
#[test]
fn lagging_statements_are_listed_with_their_period() {
    assert!(lagging_problems(Some(&[])).is_empty());
    assert_eq!(
        lagging_problems(Some(&[lagging("3169", 1), lagging("6274", 2)])),
        [
            "2 檔官方季報已公布 7 天以上、損益表還沒有那一季：3169 2026Q2（10-01 公布）、6274 2026Q2（10-02 公布）"
        ]
    );
    let many: Vec<LaggingStatement> = (0..12)
        .map(|i| lagging(&format!("{}", 1101 + i), 1))
        .collect();
    let problems = lagging_problems(Some(&many));
    assert!(problems[0].starts_with("12 檔"), "{problems:?}");
    assert!(
        problems[0].ends_with("1110 2026Q2（10-01 公布）、…共 12 檔"),
        "{problems:?}"
    );
    assert_eq!(lagging_problems(None), ["查詢失敗，見 warn 日誌"]);
}

/// 回傳固定落後名單（`None` 時查詢失敗）的假倉儲。
struct FakeReports(Option<Vec<LaggingStatement>>);

#[async_trait::async_trait]
impl FinancialReportRepository for FakeReports {
    async fn save_income_statements(
        &self,
        _: &[crate::domain::financial::statement::IncomeStatement],
    ) -> Result<u64> {
        unimplemented!("健康檢查不寫入")
    }

    async fn save_balance_sheets(
        &self,
        _: &[crate::domain::financial::statement::BalanceSheet],
    ) -> Result<u64> {
        unimplemented!("健康檢查不寫入")
    }

    async fn save_cash_flow_statements(
        &self,
        _: &[crate::domain::financial::statement::CashFlowStatement],
    ) -> Result<u64> {
        unimplemented!("健康檢查不寫入")
    }

    async fn fetch_lagging_statements(
        &self,
        published_within_days: i32,
        min_age_days: i32,
    ) -> Result<Vec<LaggingStatement>> {
        assert_eq!(
            (published_within_days, min_age_days),
            (LAGGING_LOOKBACK_DAYS, LAGGING_MIN_AGE_DAYS)
        );
        self.0.clone().ok_or_else(|| anyhow::anyhow!("db down"))
    }
}

/// 財報落後項目以一年內、公布滿 7 天查詢；查詢失敗列為問題。
#[tokio::test]
async fn lagging_check_queries_stale_reports() {
    let check = lagging_check(&FakeReports(Some(vec![lagging("3169", 1)]))).await;
    assert_eq!(check.name, "財報落後");
    assert_eq!(check.problems.len(), 1);
    assert!(
        lagging_check(&FakeReports(Some(Vec::new())))
            .await
            .problems
            .is_empty()
    );
    assert_eq!(
        lagging_check(&FakeReports(None)).await.problems,
        ["查詢失敗，見 warn 日誌"]
    );
}
