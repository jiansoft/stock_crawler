//! 對外入口：單檔 Yahoo／BigGo 回補（mock 來源＋假倉儲）、股票間隔。

use wiremock::matchers::{method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::core::util::http::mock_origin::with_origins;

const YAHOO: &str = "https://tw.stock.yahoo.com";
const BIGGO: &str = "https://api.biggo.com";

/// 股票之間停 1.5～3 秒。
#[tokio::test(start_paused = true)]
async fn stock_interval_waits_one_and_a_half_to_three_seconds() {
    let started = tokio::time::Instant::now();
    stock_interval().await;
    let waited = started.elapsed();
    assert!(
        (Duration::from_millis(1500)..=Duration::from_millis(3000)).contains(&waited),
        "{waited:?}"
    );
}

async fn mock_json(server: &MockServer, mock: wiremock::MockBuilder, body: &str) {
    mock.respond_with(ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json"))
        .mount(server)
        .await;
}

/// 單檔 Yahoo 回補：五支 endpoint 都成功時三張表都寫入，回傳各表列數。
#[tokio::test]
async fn backfill_for_stock_saves_three_yahoo_tables() {
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
        let mock = Mock::given(method("GET")).and(path_regex(format!(
            r"^/_td-stock/api/resource/StockServices\.{service};.*period={period};"
        )));
        mock_json(&server, mock, body).await;
    }
    let repo = FakeRepository::default();

    let counts = with_origins(
        &[(YAHOO, &server.uri())],
        backfill_for_stock(&repo, "8042.TWO"),
    )
    .await
    .expect("五支 endpoint 都成功");

    assert!(
        counts.income_statements > 0
            && counts.balance_sheets > 0
            && counts.cash_flow_statements > 0
    );
    let tables: Vec<String> = repo
        .saved
        .lock()
        .unwrap()
        .iter()
        .map(|(table, _)| table.clone())
        .collect();
    assert_eq!(tables, ["income", "balance", "cash_flow"]);
}

/// 單檔 BigGo 回補：九支 endpoint 都成功時寫入三張表；第一支就失敗時回錯誤、不寫入。
#[tokio::test]
async fn backfill_from_biggo_saves_or_fails_before_writing() {
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
        let mut mock = Mock::given(method("GET")).and(path(format!("/api/v1/finance{route}")));
        if let Some(kind) = kind {
            mock = mock.and(query_param("type", kind));
        }
        mock_json(&server, mock, body).await;
    }
    let repo = FakeRepository::default();
    let counts = with_origins(
        &[(BIGGO, &server.uri())],
        backfill_from_biggo(&repo, "2330"),
    )
    .await
    .expect("九支 endpoint 都成功");
    assert!(counts.income_statements > 0 && counts.balance_sheets > 0);

    let broken = MockServer::start().await;
    let untouched = FakeRepository::default();
    let error = with_origins(
        &[(BIGGO, &broken.uri())],
        backfill_from_biggo(&untouched, "2330"),
    )
    .await
    .expect_err("BigGo 讀不到要回錯誤");
    assert!(error.to_string().contains("2330"), "{error:#}");
    assert!(
        untouched.saved.lock().unwrap().is_empty(),
        "請求失敗時不寫入"
    );
}
