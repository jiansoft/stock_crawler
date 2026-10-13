//! MOPS 年度財報採集器的解析與計算測試。

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
