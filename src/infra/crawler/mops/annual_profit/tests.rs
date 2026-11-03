//! MOPS 年度財報採集器的解析與計算測試。

use super::parse::parse_year_from_xaxis;
use super::*;

#[test]
fn test_parse_year_from_xaxis() {
    assert_eq!(parse_year_from_xaxis("2024Q4").unwrap(), 2024);
}

#[test]
fn test_parse_year_from_xaxis_rejects_malformed_values() {
    assert!(parse_year_from_xaxis("").is_err());
    assert!(parse_year_from_xaxis("202").is_err());
    assert!(parse_year_from_xaxis("202Q4").is_err());
    assert!(parse_year_from_xaxis("YEARQ4").is_err());
}

#[test]
fn test_calc_sales_per_share() {
    let revenue = Decimal::from_str_exact("2894307699").unwrap();
    let common_stock = Decimal::from_str_exact("259327332").unwrap();

    assert_eq!(
        calc_sales_per_share(revenue, common_stock).round_dp(2),
        Decimal::from_str_exact("111.61").unwrap()
    );
}

#[test]
fn test_calc_sales_per_share_returns_zero_when_common_stock_is_zero() {
    let revenue = Decimal::from_str_exact("2894307699").unwrap();

    assert_eq!(calc_sales_per_share(revenue, Decimal::ZERO), Decimal::ZERO);
}

#[test]
fn test_round_per_share_values_to_two_decimal_places() {
    let sales_per_share = Decimal::from_str_exact("4.6650656274625312897574062788")
        .unwrap()
        .round_dp(2);
    let earnings_per_share = Decimal::from_str_exact("1.1599999999999999200639422265")
        .unwrap()
        .round_dp(2);

    assert_eq!(sales_per_share, Decimal::from_str_exact("4.67").unwrap());
    assert_eq!(earnings_per_share, Decimal::from_str_exact("1.16").unwrap());
}

#[test]
fn test_round_per_share_values_uses_midpoint_to_even() {
    let positive = Decimal::from_str_exact("12.345").unwrap().round_dp(2);
    let negative = Decimal::from_str_exact("-12.345").unwrap().round_dp(2);

    assert_eq!(positive, Decimal::from_str_exact("12.34").unwrap());
    assert_eq!(negative, Decimal::from_str_exact("-12.34").unwrap());
}

#[test]
fn test_calc_weighted_average_shares() {
    let profit_for_eps_total = Decimal::from_str_exact("1717882.627").unwrap();
    let earnings_per_share = Decimal::from_str_exact("66.26").unwrap();

    assert_eq!(
        calc_weighted_average_shares(profit_for_eps_total, earnings_per_share).round_dp(3),
        Decimal::from_str_exact("25926.390").unwrap()
    );
}

#[test]
fn test_calc_weighted_average_shares_returns_zero_when_eps_is_zero() {
    let profit_for_eps_total = Decimal::from_str_exact("1717882.627").unwrap();

    assert_eq!(
        calc_weighted_average_shares(profit_for_eps_total, Decimal::ZERO),
        Decimal::ZERO
    );
}

#[test]
fn test_calc_profit_before_tax_per_share() {
    let total_profit_before_tax = Decimal::from_str_exact("2041663").unwrap();
    let weighted_average_shares = Decimal::from_str_exact("25926.390").unwrap();

    assert_eq!(
        calc_profit_before_tax_per_share(total_profit_before_tax, weighted_average_shares)
            .round_dp(2),
        Decimal::from_str_exact("78.75").unwrap()
    );
}

#[test]
fn test_calc_profit_before_tax_per_share_returns_zero_when_weighted_shares_is_zero() {
    let total_profit_before_tax = Decimal::from_str_exact("2041663").unwrap();

    assert_eq!(
        calc_profit_before_tax_per_share(total_profit_before_tax, Decimal::ZERO),
        Decimal::ZERO
    );
}

#[test]
fn test_parse_income_statement_metrics_from_report() {
    let html = r#"
        <div id="headTable">
            <table>
                <tbody>
                    <tr><td>營業收入合計</td></tr>
                    <tr><td>稅前淨利（淨損）</td></tr>
                    <tr><td>本期淨利（淨損）</td></tr>
                </tbody>
            </table>
        </div>
        <div id="bodyTable">
            <table>
                <tbody>
                    <tr><td>3,809,054,272</td></tr>
                    <tr><td>1,700,000,000</td></tr>
                    <tr><td>1,400,000,000</td></tr>
                </tbody>
            </table>
        </div>
    "#;

    let metrics = parse_income_statement_metrics_from_report(html).unwrap();

    assert_eq!(
        metrics.profit_before_tax_total,
        Decimal::from_str_exact("1700000000").unwrap()
    );
    assert_eq!(
        metrics.profit_for_eps_total,
        Decimal::from_str_exact("1400000000").unwrap()
    );
}

#[test]
fn test_parse_income_statement_metrics_from_financial_report() {
    let html = r#"
        <div id="headTable">
            <table>
                <tbody>
                    <tr><td>淨收益</td></tr>
                    <tr><td>繼續營業單位稅前淨利（淨損）</td></tr>
                    <tr><td>本期稅後淨利（淨損）</td></tr>
                </tbody>
            </table>
        </div>
        <div id="bodyTable">
            <table>
                <tbody>
                    <tr><td>60,000,000</td></tr>
                    <tr><td>18,765,432</td></tr>
                    <tr><td>15,000,000</td></tr>
                </tbody>
            </table>
        </div>
    "#;

    let metrics = parse_income_statement_metrics_from_report(html).unwrap();

    assert_eq!(
        metrics.profit_before_tax_total,
        Decimal::from_str_exact("18765432").unwrap()
    );
    assert_eq!(
        metrics.profit_for_eps_total,
        Decimal::from_str_exact("15000000").unwrap()
    );
}

#[test]
fn test_parse_income_statement_metrics_prefers_profit_for_eps_over_net_profit() {
    let html = r#"
        <div id="headTable">
            <table>
                <tbody>
                    <tr><td>繼續營業單位稅前損益</td></tr>
                    <tr><td>本期淨利（淨損）</td></tr>
                    <tr><td>母公司業主（淨利／損）</td></tr>
                </tbody>
            </table>
        </div>
        <div id="bodyTable">
            <table>
                <tbody>
                    <tr><td>20,000,000</td></tr>
                    <tr><td>15,000,000</td></tr>
                    <tr><td>12,500,000</td></tr>
                </tbody>
            </table>
        </div>
    "#;

    let metrics = parse_income_statement_metrics_from_report(html).unwrap();

    assert_eq!(
        metrics.profit_before_tax_total,
        Decimal::from_str_exact("20000000").unwrap()
    );
    assert_eq!(
        metrics.profit_for_eps_total,
        Decimal::from_str_exact("12500000").unwrap()
    );
}

#[test]
fn test_parse_income_statement_metrics_from_empty_report() {
    let html = r#"
        <div id="headTable">
            <table><tbody></tbody></table>
        </div>
        <div id="bodyTable">
            <table><tbody></tbody></table>
        </div>
    "#;

    let metrics = parse_income_statement_metrics_from_report(html).unwrap();

    assert_eq!(metrics.profit_before_tax_total, Decimal::ZERO);
    assert_eq!(metrics.profit_for_eps_total, Decimal::ZERO);
}

#[test]
fn test_parse_income_statement_metrics_from_report_with_missing_body_values() {
    let html = r#"
        <div id="headTable">
            <table>
                <tbody>
                    <tr><td>稅前淨利（淨損）</td></tr>
                    <tr><td>本期淨利（淨損）</td></tr>
                </tbody>
            </table>
        </div>
        <div id="bodyTable">
            <table><tbody></tbody></table>
        </div>
    "#;

    let metrics = parse_income_statement_metrics_from_report(html).unwrap();

    assert_eq!(metrics.profit_before_tax_total, Decimal::ZERO);
    assert_eq!(metrics.profit_for_eps_total, Decimal::ZERO);
}

#[test]
fn test_parse_income_statement_metrics_returns_error_for_malformed_number() {
    let html = r#"
        <div id="headTable">
            <table>
                <tbody>
                    <tr><td>稅前淨利（淨損）</td></tr>
                </tbody>
            </table>
        </div>
        <div id="bodyTable">
            <table>
                <tbody>
                    <tr><td>not-a-number</td></tr>
                </tbody>
            </table>
        </div>
    "#;

    assert!(parse_income_statement_metrics_from_report(html).is_err());
}

/// 趨勢資料轉為年度對照表；null 資料點略過。
#[test]
fn parse_compare_series_maps_years_and_skips_nulls() {
    let raw = r#"{"xaxisList":["2022Q4","2023Q4","2024Q4"],
        "graphData":[{"data":[[0,13.94,"A"],[1,null,"A"],[2,45.25,"A"]]}]}"#;
    let series = parse_compare_series(raw, "EPS").expect("解析成功");
    assert_eq!(series.len(), 2);
    assert_eq!(
        series[&2022].round_dp(2),
        Decimal::from_str_exact("13.94").unwrap()
    );
    assert_eq!(series[&2024], Decimal::from_str_exact("45.25").unwrap());
    assert!(!series.contains_key(&2023));
}

/// JSON 格式錯誤、沒有 graphData、X 軸索引越界或年度格式錯誤都回錯誤。
#[test]
fn parse_compare_series_rejects_malformed_responses() {
    for (raw, expected) in [
        (
            "<html>維護中</html>",
            "Failed to parse MOPS compare/data response",
        ),
        (
            r#"{"xaxisList":[],"graphData":[]}"#,
            "Failed to find graphData",
        ),
        (
            r#"{"xaxisList":["2024Q4"],"graphData":[{"data":[[3,1.0,"A"]]}]}"#,
            "Failed to find xaxis index 3",
        ),
        (
            r#"{"xaxisList":["年度"],"graphData":[{"data":[[0,1.0,"A"]]}]}"#,
            "",
        ),
    ] {
        let error = parse_compare_series(raw, "EPS").expect_err(raw);
        assert!(error.to_string().contains(expected), "{error}");
    }
}

/// 組合年度財報：有營收、股本與損益表時算出每股值；缺值時以 0 代替。
#[test]
fn assemble_annual_profit_uses_available_metrics() {
    let d = |value: &str| Decimal::from_str_exact(value).unwrap();
    let full = assemble_annual_profit(
        "2330",
        2024,
        d("10.005"),
        Some(d("1000000")),
        Some(d("100000")),
        Some(IncomeStatementMetrics {
            profit_before_tax_total: d("1200000"),
            profit_for_eps_total: d("1000500"),
        }),
    );
    assert_eq!(full.stock_symbol, "2330");
    assert_eq!(full.year, 2024);
    assert_eq!(full.earnings_per_share, d("10.00"));
    assert_eq!(full.sales_per_share, d("100.00"));
    assert_eq!(full.profit_before_tax, d("12.00"));

    let sparse = assemble_annual_profit("2330", 2023, d("5"), Some(d("1000")), None, None);
    assert_eq!(sparse.sales_per_share, Decimal::ZERO);
    assert_eq!(sparse.profit_before_tax, Decimal::ZERO);
    assert_eq!(sparse.earnings_per_share, d("5"));
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 mops::annual_profit::visit");

    for stock_symbol in ["2330", "2838"] {
        match visit(stock_symbol).await {
            Ok(result) => {
                dbg!(&result);
                tracing::debug!("mops : {:#?}", result);
            }
            Err(why) => {
                tracing::debug!(
                    "Failed to mops::annual_profit::visit({}) because {:?}",
                    stock_symbol,
                    why
                );
                panic!(
                    "mops::annual_profit::visit({}) failed: {:?}",
                    stock_symbol, why
                );
            }
        }
    }

    tracing::debug!("結束 mops::annual_profit::visit");
}

/// 以本機 mock server 取代 MOPS（C18，沿用 G15 的 mock_origin）：EPS、營收、股本三條趨勢資料
/// 加上逐年的綜合損益表，組出年度財報。
mod mock_sources {
    use rust_decimal_macros::dec;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;

    /// `compare/data` 的回應：2023、2024 兩年各一個值。
    fn series(values: [f64; 2]) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "xaxisList": ["2023Q4", "2024Q4"],
            "graphData": [{ "data": [[0, values[0], "A"], [1, values[1], "A"]] }]
        }))
    }

    /// 2024 年的綜合損益表：稅前淨利 1,000,000 仟元、母公司業主淨利 800,000 仟元。
    const REPORT_2024: &str = r#"
        <div id="headTable"><table><tbody>
            <tr><td>稅前淨利（淨損）</td></tr>
            <tr><td>本期淨利（淨損）</td></tr>
            <tr><td>母公司業主（淨利／損）</td></tr>
        </tbody></table></div>
        <div id="bodyTable"><table><tbody>
            <tr><td>1,000,000</td></tr>
            <tr><td>820,000</td></tr>
            <tr><td>800,000</td></tr>
        </tbody></table></div>
    "#;

    async fn mount_series(server: &MockServer, item: &str, values: [f64; 2]) {
        Mock::given(method("POST"))
            .and(path("/compare/data"))
            .and(body_string_contains(format!("compareItem={item}")))
            .respond_with(series(values))
            .mount(server)
            .await;
    }

    async fn visit_mocked(server: &MockServer) -> Result<Vec<share::AnnualProfit>> {
        with_origins(
            &[("https://mopsfin.twse.com.tw", &server.uri())],
            visit("2330"),
        )
        .await
    }

    /// 依年度排序；2024 年有綜合損益表算出每股稅前淨利，2023 年抓不到就留 0。
    #[tokio::test]
    async fn visit_assembles_annual_profits_from_series_and_reports() {
        let server = MockServer::start().await;
        mount_series(&server, "EPS", [32.34, 40.0]).await;
        mount_series(&server, "Revenue", [2_000_000.0, 3_000_000.0]).await;
        mount_series(&server, "CommonStock", [100_000.0, 100_000.0]).await;
        Mock::given(method("POST"))
            .and(path("/compare/report"))
            .and(body_string_contains("ys=20244"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(REPORT_2024, "text/html; charset=utf-8"),
            )
            .mount(&server)
            .await;

        let profits = visit_mocked(&server).await.expect("MOPS 有資料");

        let years: Vec<i32> = profits.iter().map(|profit| profit.year).collect();
        assert_eq!(years, [2023, 2024]);
        assert_eq!(profits[0].earnings_per_share, dec!(32.34));
        assert_eq!(
            profits[0].profit_before_tax,
            Decimal::ZERO,
            "2023 年沒有損益表"
        );
        assert!(profits[0].sales_per_share > Decimal::ZERO);
        assert!(profits[1].profit_before_tax > Decimal::ZERO);
    }

    /// EPS 沒有任何資料點時直接回錯誤，不再查營收與股本。
    #[tokio::test]
    async fn visit_fails_when_eps_series_is_empty() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/compare/data"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "xaxisList": [],
                "graphData": [{ "data": [] }]
            })))
            .mount(&server)
            .await;

        let error = visit_mocked(&server).await.expect_err("EPS 是空的");

        assert!(error.to_string().contains("response is empty"), "{error:#}");
        let requests = server.received_requests().await.unwrap_or_default();
        assert_eq!(requests.len(), 1, "只查 EPS");
    }
}
