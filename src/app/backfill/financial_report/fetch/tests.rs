//! 財報抓取的測試：Yahoo 與 BigGo 結果組裝、遇到壞列失敗，以及 mock 端點下五支 Yahoo／九支 BigGo 的抓取。

use rust_decimal_macros::dec;

use super::*;
use crate::infra::crawler::yahoo::financial_statement::FiscalPeriod as YahooPeriod;

fn yahoo<T>(
    period: ReportPeriod,
    year: i32,
    quarter: Option<u8>,
    items: T,
) -> FinancialStatement<T> {
    FinancialStatement {
        stock_symbol: "2330".to_owned(),
        report_period: period,
        fiscal_period: YahooPeriod { year, quarter },
        items,
    }
}

fn biggo<T>(kind: ReportKind, year: i32, quarter: Option<u8>, items: T) -> Statement<T> {
    Statement {
        stock_symbol: "2330".to_owned(),
        kind,
        period: FiscalPeriod { year, quarter },
        items,
    }
}

/// Yahoo：單季與年度損益表、現金流量表都要轉譯，資產負債表只有單季；金額換算成千元。
#[test]
fn assemble_yahoo_maps_every_row() {
    let raw = YahooRaw {
        income_statements: vec![
            yahoo(
                ReportPeriod::Quarter,
                2026,
                Some(2),
                IncomeStatementItems {
                    revenue: Some(dec!(5_000_000)),
                    eps: Some(dec!(1.25)),
                    ..Default::default()
                },
            ),
            yahoo(
                ReportPeriod::Year,
                2025,
                None,
                IncomeStatementItems::default(),
            ),
        ],
        cash_flow_statements: vec![yahoo(
            ReportPeriod::Year,
            2025,
            None,
            CashFlowItems::default(),
        )],
        balance_sheets: vec![yahoo(
            ReportPeriod::Quarter,
            2026,
            Some(2),
            BalanceSheetItems::default(),
        )],
    };
    let reports = assemble_yahoo(&raw).expect("可轉譯");
    assert_eq!(reports.income_statements.len(), 2);
    assert_eq!(reports.income_statements[0].revenue, Some(5_000));
    assert_eq!(reports.cash_flow_statements.len(), 1);
    assert_eq!(reports.balance_sheets.len(), 1);
    assert!(!reports.is_empty());

    assert!(
        assemble_yahoo(&YahooRaw::default())
            .expect("空的也可轉譯")
            .is_empty()
    );
}

/// Yahoo：任一列轉譯失敗（金額無法整除 1000、資產負債表不是單季）整檔失敗。
#[test]
fn assemble_yahoo_fails_on_any_bad_row() {
    let fractional = YahooRaw {
        income_statements: vec![yahoo(
            ReportPeriod::Quarter,
            2026,
            Some(2),
            IncomeStatementItems {
                revenue: Some(dec!(1_234_567)),
                ..Default::default()
            },
        )],
        ..Default::default()
    };
    assert!(assemble_yahoo(&fractional).is_err());

    let annual_balance = YahooRaw {
        balance_sheets: vec![yahoo(
            ReportPeriod::Year,
            2025,
            None,
            BalanceSheetItems::default(),
        )],
        ..Default::default()
    };
    assert!(assemble_yahoo(&annual_balance).is_err());
}

/// BigGo：年度資料只保留已有第四季單季的年度（今年至今的累計不寫入）。
#[test]
fn assemble_biggo_keeps_only_completed_years() {
    let raw = BigGoRaw {
        single_income: vec![
            biggo(ReportKind::Single, 2026, Some(1), IncomeItems::default()),
            biggo(ReportKind::Single, 2025, Some(4), IncomeItems::default()),
        ],
        annual_income: vec![
            biggo(ReportKind::Annual, 2026, None, IncomeItems::default()),
            biggo(ReportKind::Annual, 2025, None, IncomeItems::default()),
        ],
        single_cash_flow: vec![biggo(
            ReportKind::Single,
            2026,
            Some(1),
            BigGoCashFlowItems::default(),
        )],
        annual_cash_flow: vec![biggo(
            ReportKind::Annual,
            2025,
            None,
            BigGoCashFlowItems::default(),
        )],
        balance_sheets: vec![biggo(
            ReportKind::Single,
            2026,
            Some(1),
            BalanceItems::default(),
        )],
    };
    let reports = assemble_biggo(raw).expect("可轉譯");
    // 兩筆單季＋2025 年度；2026 年度沒有 Q4，不保留。
    assert_eq!(reports.income_statements.len(), 3);
    // 現金流量表單季沒有 2025-Q4，2025 年度也不保留。
    assert_eq!(reports.cash_flow_statements.len(), 1);
    assert_eq!(reports.balance_sheets.len(), 1);
}

mod mock_fetch {
    use wiremock::matchers::{method, path, path_regex, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::super::*;
    use crate::core::util::http::mock_origin::with_origins;

    const YAHOO: &str = "https://tw.stock.yahoo.com";
    const BIGGO: &str = "https://api.biggo.com";

    async fn yahoo_mock(server: &MockServer, service: &str, period: &str, body: &str) {
        Mock::given(method("GET"))
            .and(path_regex(format!(
                r"^/_td-stock/api/resource/StockServices\.{service};.*period={period};"
            )))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json"),
            )
            .expect(1)
            .mount(server)
            .await;
    }

    /// Yahoo 5 支 endpoint（損益與現金流量各季／年、資產負債表單季）全部成功：三張表都有資料。
    #[tokio::test]
    async fn fetch_reports_collects_five_yahoo_endpoints() {
        let server = MockServer::start().await;
        let fixtures = [
            (
                "incomeStatements-growthAnalyses",
                "quarter",
                include_str!(
                    "../../../../infra/crawler/yahoo/testdata/financial_income_statement_quarter_sum_8042.json"
                ),
            ),
            (
                "incomeStatements-growthAnalyses",
                "year",
                include_str!(
                    "../../../../infra/crawler/yahoo/testdata/financial_income_statement_year_8042.json"
                ),
            ),
            (
                "cashFlowStatements",
                "quarter",
                include_str!(
                    "../../../../infra/crawler/yahoo/testdata/financial_cash_flow_quarter_8042.json"
                ),
            ),
            (
                "cashFlowStatements",
                "year",
                include_str!(
                    "../../../../infra/crawler/yahoo/testdata/financial_cash_flow_year_8042.json"
                ),
            ),
            (
                "balanceSheets",
                "quarter",
                include_str!(
                    "../../../../infra/crawler/yahoo/testdata/financial_balance_sheet_quarter_8042.json"
                ),
            ),
        ];
        for (service, period, body) in fixtures {
            yahoo_mock(&server, service, period, body).await;
        }

        let reports = with_origins(&[(YAHOO, &server.uri())], fetch_reports("8042.TWO"))
            .await
            .expect("五支 endpoint 都成功");

        assert!(!reports.is_empty());
        assert!(!reports.income_statements.is_empty());
        assert!(!reports.cash_flow_statements.is_empty());
        assert!(!reports.balance_sheets.is_empty());
    }

    /// 第一支（單季損益表）就失敗：回錯誤並標明是哪一步，不再發後面的請求。
    #[tokio::test]
    async fn fetch_reports_stops_at_first_failed_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad"))
            .expect(1)
            .mount(&server)
            .await;

        let message = with_origins(&[(YAHOO, &server.uri())], fetch_reports("8042.TWO"))
            .await
            .err()
            .expect("第一步失敗要回錯誤")
            .to_string();
        assert!(
            message.contains("fetch income statements failed: 8042.TWO Quarter"),
            "{message}"
        );
    }

    async fn biggo_mock(server: &MockServer, route: &str, kind: Option<&str>, body: &str) {
        let mut mock = Mock::given(method("GET")).and(path(format!("/api/v1/finance{route}")));
        if let Some(kind) = kind {
            mock = mock.and(query_param("type", kind));
        }
        mock.respond_with(
            ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json"),
        )
        .expect(1)
        .mount(server)
        .await;
    }

    /// BigGo 9 支 endpoint 全部成功：損益、現金流量、資產負債表都轉譯出資料。
    #[tokio::test]
    async fn fetch_biggo_reports_collects_nine_endpoints() {
        let server = MockServer::start().await;
        let fixtures = [
            (
                "/stock/statements/profit",
                Some("single"),
                include_str!("../../../../infra/crawler/biggo/testdata/profit_single_2330.json"),
            ),
            (
                "/stock/statements/eps",
                Some("single"),
                include_str!("../../../../infra/crawler/biggo/testdata/eps_single_2330.json"),
            ),
            (
                "/stock/statements/profit",
                Some("annual"),
                include_str!("../../../../infra/crawler/biggo/testdata/profit_annual_2330.json"),
            ),
            (
                "/stock/statements/eps",
                Some("annual"),
                include_str!("../../../../infra/crawler/biggo/testdata/eps_annual_2330.json"),
            ),
            (
                "/stock/statements/cash-flow",
                Some("single"),
                include_str!("../../../../infra/crawler/biggo/testdata/cash_flow_single_2330.json"),
            ),
            (
                "/stock/statements/cash-flow",
                Some("annual"),
                include_str!("../../../../infra/crawler/biggo/testdata/cash_flow_annual_2330.json"),
            ),
            (
                "/stock/statements/total-assets",
                None,
                include_str!("../../../../infra/crawler/biggo/testdata/total_assets_2330.json"),
            ),
            (
                "/stock/statements/liabilities",
                None,
                include_str!("../../../../infra/crawler/biggo/testdata/liabilities_2330.json"),
            ),
            (
                "/stock/statements/equity",
                None,
                include_str!("../../../../infra/crawler/biggo/testdata/equity_2330.json"),
            ),
        ];
        for (route, kind, body) in fixtures {
            biggo_mock(&server, route, kind, body).await;
        }

        let reports = with_origins(&[(BIGGO, &server.uri())], fetch_biggo_reports("2330"))
            .await
            .expect("九支 endpoint 都成功");

        assert!(!reports.income_statements.is_empty());
        assert!(!reports.cash_flow_statements.is_empty());
        assert!(!reports.balance_sheets.is_empty());
    }

    /// BigGo 第一支就失敗：錯誤標明是單季損益表。
    #[tokio::test]
    async fn fetch_biggo_reports_reports_failed_step() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad"))
            .mount(&server)
            .await;

        let message = with_origins(&[(BIGGO, &server.uri())], fetch_biggo_reports("2330"))
            .await
            .err()
            .expect("第一步失敗要回錯誤")
            .to_string();
        assert!(
            message.contains("fetch BigGo single income statements failed: 2330"),
            "{message}"
        );
    }
}
