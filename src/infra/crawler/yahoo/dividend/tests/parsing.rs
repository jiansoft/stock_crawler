//! 股利頁 HTML 解析：依發放年度分組排序、缺欄位與格式錯誤、半年配的發放年度與錯誤分類。

use super::*;

#[test]
fn parse_dividend_html_groups_records_by_paid_year_and_sorts_desc() {
    let html = wrap_rows(&[
        dividend_row("2025", "42.00", "-", "尚未公布", "-", "尚未公布", "-"),
        dividend_row(
            "2024Q4",
            "1.5",
            "0.5",
            "2025/03/10",
            "-",
            "2025/04/11",
            "2025/05/01",
        ),
        dividend_row("2024Q3", "-", "1.2", "-", "2025/01/15", "-", "2025/02/20"),
        dividend_row("2023", "2.0", "-", "2024/07/01", "-", "2024/07/30", "-"),
        dividend_row("2022Q2", "1.0", "0.0", "-", "-", "-", "-"),
    ]);

    let dividend = parse_dividend_html("2330", "https://example.test/quote/2330/dividend", &html)
        .expect("expected parser to extract dividend rows");

    assert_eq!(dividend.stock_symbol, "2330");
    assert_eq!(dividend.dividend.len(), 4);
    assert_eq!(dividend.dividend[0].0, 2026);
    assert_eq!(dividend.dividend[1].0, 2025);
    assert_eq!(dividend.dividend[2].0, 2024);
    assert_eq!(dividend.dividend[3].0, 2023);

    let details_2026 = dividend
        .get_dividend_by_year(2026)
        .expect("expected estimated grouped data for 2026");
    assert_eq!(details_2026.len(), 1);
    let announced = &details_2026[0];
    assert_eq!(announced.year, 2026);
    assert_eq!(announced.year_of_dividend, 2025);
    assert_eq!(announced.quarter, "");
    assert_eq!(announced.cash_dividend, dec!(42.00));
    assert_eq!(announced.stock_dividend, Decimal::ZERO);
    assert_eq!(announced.ex_dividend_date1, "尚未公布");
    assert_eq!(announced.ex_dividend_date2, "-");
    assert_eq!(announced.payable_date1, "尚未公布");
    assert_eq!(announced.payable_date2, "-");

    let details_2025 = dividend
        .get_dividend_by_year(2025)
        .expect("expected grouped data for 2025");
    assert_eq!(details_2025.len(), 2);

    let q4 = &details_2025[0];
    assert_eq!(q4.year, 2025);
    assert_eq!(q4.year_of_dividend, 2024);
    assert_eq!(q4.quarter, "Q4");
    assert_eq!(q4.cash_dividend, dec!(1.5));
    assert_eq!(q4.stock_dividend, dec!(0.5));
    assert_eq!(q4.ex_dividend_date1, "2025-03-10");
    assert_eq!(q4.ex_dividend_date2, "-");
    assert_eq!(q4.payable_date1, "2025-04-11");
    assert_eq!(q4.payable_date2, "2025-05-01");

    let q3 = &details_2025[1];
    assert_eq!(q3.year, 2025);
    assert_eq!(q3.year_of_dividend, 2024);
    assert_eq!(q3.quarter, "Q3");
    assert_eq!(q3.cash_dividend, Decimal::ZERO);
    assert_eq!(q3.stock_dividend, dec!(1.2));
    assert_eq!(q3.ex_dividend_date1, "-");
    assert_eq!(q3.ex_dividend_date2, "2025-01-15");
    assert_eq!(q3.payable_date1, "-");
    assert_eq!(q3.payable_date2, "2025-02-20");

    let details_2024 = dividend
        .get_dividend_by_year(2024)
        .expect("expected grouped data for 2024");
    assert_eq!(details_2024.len(), 1);
    assert_eq!(details_2024[0].quarter, "");
    assert_eq!(details_2024[0].year_of_dividend, 2023);
    assert_eq!(details_2024[0].cash_dividend, dec!(2.0));
    assert_eq!(details_2024[0].stock_dividend, Decimal::ZERO);
    assert_eq!(details_2024[0].ex_dividend_date1, "2024-07-01");
    assert_eq!(details_2024[0].payable_date1, "2024-07-30");

    let details_2023 = dividend
        .get_dividend_by_year(2023)
        .expect("expected estimated grouped data for 2023");
    assert_eq!(details_2023.len(), 1);
    assert_eq!(details_2023[0].year, 2023);
    assert_eq!(details_2023[0].year_of_dividend, 2022);
    assert_eq!(details_2023[0].quarter, "Q2");
    assert_eq!(details_2023[0].cash_dividend, dec!(1.0));
    assert_eq!(details_2023[0].stock_dividend, Decimal::ZERO);
    assert_eq!(details_2023[0].ex_dividend_date1, "-");
    assert_eq!(details_2023[0].payable_date1, "-");

    assert!(dividend.get_dividend_by_year(2022).is_none());
}

#[test]
fn parse_dividend_html_returns_error_when_dividend_list_is_missing() {
    let err = parse_dividend_html(
        "2330",
        "https://example.test/quote/2330/dividend",
        r#"<div id="main-2-QuoteDividend-Proxy"><ul></ul></div>"#,
    )
    .expect_err("expected parser to reject empty dividend list");

    let message = err.to_string();
    assert!(message.contains("2330"));
    assert!(message.contains("https://example.test/quote/2330/dividend"));
}

#[test]
fn parse_dividend_html_skips_rows_without_period_container() {
    let html = wrap_rows(&[
        dividend_row_without_period("99.9"),
        dividend_row("2024", "3.2", "-", "2025/07/01", "-", "2025/07/31", "-"),
    ]);

    let dividend = parse_dividend_html("2330", "https://example.test/quote/2330/dividend", &html)
        .expect("expected parser to ignore malformed rows and keep valid rows");

    assert_eq!(dividend.dividend.len(), 1);
    let details = dividend
        .get_dividend_by_year(2025)
        .expect("expected valid annual row to be grouped by ex-dividend year");

    assert_eq!(details.len(), 1);
    assert_eq!(details[0].year_of_dividend, 2024);
    assert_eq!(details[0].cash_dividend, dec!(3.2));
    assert_eq!(details[0].stock_dividend, Decimal::ZERO);
}

#[test]
fn parse_dividend_html_defaults_missing_columns_and_malformed_numbers() {
    let html = wrap_rows(&[incomplete_dividend_row("2024Q2")]);

    let dividend = parse_dividend_html("2330", "https://example.test/quote/2330/dividend", &html)
        .expect("expected parser to tolerate short Yahoo rows");

    let details = dividend
        .get_dividend_by_year(2025)
        .expect("expected missing dates to fall back to estimated paid year");

    assert_eq!(details.len(), 1);
    let detail = &details[0];
    assert_eq!(detail.year, 2025);
    assert_eq!(detail.year_of_dividend, 2024);
    assert_eq!(detail.quarter, "Q2");
    assert_eq!(detail.cash_dividend, Decimal::ZERO);
    assert_eq!(detail.stock_dividend, Decimal::ZERO);
    assert_eq!(detail.ex_dividend_date1, "-");
    assert_eq!(detail.ex_dividend_date2, "-");
    assert_eq!(detail.payable_date1, "-");
    assert_eq!(detail.payable_date2, "-");
}

#[test]
fn parse_dividend_html_uses_payable_year_for_half_year_dividends() {
    let html = wrap_rows(&[
        dividend_row("2024H1", "1.1", "-", "2025/01/03", "-", "2024/12/20", "-"),
        dividend_row("2024H2", "1.2", "-", "2025/02/03", "-", "-", "2026/01/15"),
    ]);

    let dividend = parse_dividend_html("2330", "https://example.test/quote/2330/dividend", &html)
        .expect("expected half-year rows to parse");

    let details_2024 = dividend
        .get_dividend_by_year(2024)
        .expect("expected H1 cash payable date to determine paid year");
    assert_eq!(details_2024.len(), 1);
    assert_eq!(details_2024[0].quarter, "H1");
    assert_eq!(details_2024[0].cash_dividend, dec!(1.1));
    assert_eq!(details_2024[0].ex_dividend_date1, "2025-01-03");
    assert_eq!(details_2024[0].payable_date1, "2024-12-20");

    let details_2026 = dividend
        .get_dividend_by_year(2026)
        .expect("expected H2 stock payable date to determine paid year");
    assert_eq!(details_2026.len(), 1);
    assert_eq!(details_2026[0].quarter, "H2");
    assert_eq!(details_2026[0].cash_dividend, dec!(1.2));
    assert_eq!(details_2026[0].ex_dividend_date1, "2025-02-03");
    assert_eq!(details_2026[0].payable_date2, "2026-01-15");
}

#[test]
fn parse_dividend_html_keeps_input_order_within_same_paid_year() {
    let html = wrap_rows(&[
        dividend_row("2024Q1", "0.1", "-", "-", "-", "2025/04/01", "-"),
        dividend_row("2024H1", "0.2", "-", "-", "-", "2025/08/01", "-"),
        dividend_row("2024Q4", "0.4", "-", "-", "-", "2025/12/01", "-"),
    ]);

    let dividend = parse_dividend_html("2330", "https://example.test/quote/2330/dividend", &html)
        .expect("expected parser to preserve row order inside grouped year");

    let details = dividend
        .get_dividend_by_year(2025)
        .expect("expected all rows to share paid year 2025");

    assert_eq!(details.len(), 3);
    assert_eq!(details[0].quarter, "Q1");
    assert_eq!(details[1].quarter, "H1");
    assert_eq!(details[2].quarter, "Q4");
}

/// 驗證 404 型別錯誤能被 `is_page_not_found_error` 辨識，
/// 且穿透 anyhow 的 `with_context` 包裝層後仍可辨識——
/// backfill 流程就是在包了 context 之後才做判斷的。
#[test]
fn is_page_not_found_error_detects_through_context_layers() {
    let err: anyhow::Error = YahooPageNotFoundError {
        stock_symbol: "3089".to_string(),
        url: "https://tw.stock.yahoo.com/quote/3089/dividend".to_string(),
    }
    .into();
    assert!(is_page_not_found_error(&err));

    // 模擬 backfill 的 with_context 包裝。
    let wrapped = err.context("yahoo dividend fetch failed: year=2026, stock_symbol=3089");
    assert!(is_page_not_found_error(&wrapped));

    // 一般錯誤不得被誤判。
    let other = anyhow!("some other error");
    assert!(!is_page_not_found_error(&other));
}
