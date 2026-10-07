//! 驗證 Yahoo 股利解析的期間、日期與分組行為，無須連線外部服務。

use anyhow::anyhow;

use super::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// 以 2753 的實際頁面驗證整年度配股會併回 25H2，且擬定列不會蓋掉已公布的配息。
///
/// Yahoo 把 2753 的 0.5 元配股拆成獨立的「2025」列，另外又在表格最上方留了一筆
/// 日期未定的 2025H2 配股；Goodinfo 的除權息日程則是把 7.5 元現金與 0.5 元配股
/// 都掛在 25H2。這個測試釘住「與 Goodinfo 一致」的解析結果。
#[test]
fn real_page_merges_annual_stock_dividend_into_half_year_event() {
    let html = include_str!("../testdata/dividend_2753.html");
    let result = parse_dividend_html("2753", "fixture", html).unwrap();

    let details_2026 = result
        .get_dividend_by_year(2026)
        .expect("expected 2026 payout group");
    assert_eq!(details_2026.len(), 2, "{:#?}", details_2026);

    let h2 = details_2026
        .iter()
        .find(|detail| detail.quarter == "H2")
        .expect("expected 2025H2 event");
    assert_eq!(h2.year_of_dividend, 2025);
    assert_eq!(h2.cash_dividend, dec!(7.50));
    assert_eq!(h2.stock_dividend, dec!(0.50));
    assert_eq!(h2.ex_dividend_date1, "2026-07-02");
    assert_eq!(h2.ex_dividend_date2, "2026-08-26");
    assert_eq!(h2.payable_date1, "2026-07-30");

    let h1 = details_2026
        .iter()
        .find(|detail| detail.quarter == "H1")
        .expect("expected 2025H1 event");
    assert_eq!(h1.cash_dividend, dec!(4.00));
    assert_eq!(h1.ex_dividend_date1, "2025-12-24");
    assert_eq!(h1.payable_date1, "2026-01-22");

    // 2026 年發放的股利合計：現金 11.5、股票 0.5，全年合計 12。
    assert_eq!(
        details_2026
            .iter()
            .map(|detail| detail.cash_dividend)
            .sum::<Decimal>(),
        dec!(11.50)
    );
    assert_eq!(
        details_2026
            .iter()
            .map(|detail| detail.stock_dividend)
            .sum::<Decimal>(),
        dec!(0.50)
    );

    // 無配息年度的空白列會被推估到隔年，若不擋掉就會和 2017 的真實配息撞同一組主鍵。
    let details_2017 = result
        .get_dividend_by_year(2017)
        .expect("expected 2017 payout group");
    assert_eq!(details_2017.len(), 1, "{:#?}", details_2017);
    assert_eq!(details_2017[0].cash_dividend, dec!(2.50));
    assert_eq!(details_2017[0].stock_dividend, dec!(1.76));
}

/// 驗證年配轉半年配保留兩筆事件，空期間合計列不可產生西元 1 年的假資料。
#[test]
fn mixed_annual_and_half_year_preserves_events_and_skips_total() {
    let html = wrap_rows(&[
        dividend_row("", "13.0833", "-", "-", "-", "-", "-"),
        dividend_row("2026H1", "6.00", "-", "2026/08/28", "-", "2026/09/29", "-"),
        dividend_row("2025", "7.0833", "-", "2026/04/10", "-", "2026/04/30", "-"),
    ]);
    let result = parse_dividend_html("2072", "fixture", &html).unwrap();
    assert_eq!(result.dividend.len(), 1);
    let details = result.get_dividend_by_year(2026).unwrap();
    assert_eq!(details.len(), 2);
    assert_eq!(details[0].quarter, "H1");
    assert_eq!(details[0].ex_dividend_date1, "2026-08-28");
    assert_eq!(details[0].payable_date1, "2026-09-29");
    assert_eq!(details[1].quarter, "A");
    assert_eq!(details[1].year_of_dividend, 2025);
    assert_eq!(details[1].ex_dividend_date1, "2026-04-10");
    assert_eq!(
        details.iter().map(|d| d.cash_dividend).sum::<Decimal>(),
        dec!(13.0833)
    );
}

fn dividend_row(
    period: &str,
    cash_dividend: &str,
    stock_dividend: &str,
    ex_dividend_date: &str,
    ex_rights_date: &str,
    payable_date1: &str,
    payable_date2: &str,
) -> String {
    format!(
        r#"
        <li>
            <div>
                <div class="Fxg(1) Fxs(1) Fxb(0%) Ta(end)">{period}</div>
                <div>unused</div>
                <div>{cash_dividend}</div>
                <div>{stock_dividend}</div>
                <div>unused</div>
                <div>unused</div>
                <div>{ex_dividend_date}</div>
                <div>{ex_rights_date}</div>
                <div>{payable_date1}</div>
                <div>{payable_date2}</div>
            </div>
        </li>
        "#
    )
}

fn wrap_rows(rows: &[String]) -> String {
    format!(
        r#"<div id="main-2-QuoteDividend-Proxy"><ul>{}</ul></div>"#,
        rows.join("")
    )
}

fn dividend_row_without_period(cash_dividend: &str) -> String {
    format!(
        r#"
        <li>
            <div>
                <div>missing-period</div>
                <div>unused</div>
                <div>{cash_dividend}</div>
                <div>0.3</div>
                <div>unused</div>
                <div>unused</div>
                <div>2025/07/01</div>
                <div>-</div>
                <div>2025/07/31</div>
                <div>-</div>
            </div>
        </li>
        "#
    )
}

fn incomplete_dividend_row(period: &str) -> String {
    format!(
        r#"
        <li>
            <div>
                <div class="Fxg(1) Fxs(1) Fxb(0%) Ta(end)">{period}</div>
                <div>unused</div>
                <div>not-a-number</div>
            </div>
        </li>
        "#
    )
}

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

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 visit");

    match visit("2357").await {
        Ok(e) => {
            dbg!(&e);
        }
        Err(why) => {
            dbg!(&why);
        }
    }

    tracing::debug!("結束 visit");
}

fn periods(result: &YahooDividend, paid_year: i32) -> Vec<(String, Decimal, String)> {
    let mut rows: Vec<(String, Decimal, String)> = result
        .get_dividend_by_year(paid_year)
        .expect("發放年度應存在")
        .iter()
        .map(|d| {
            (
                d.quarter.clone(),
                d.cash_dividend,
                d.ex_dividend_date1.clone(),
            )
        })
        .collect();
    rows.sort();
    rows
}

/// 月配 ETF 的 `2026M9` 要解析成 M09，而不是被丟掉變成空期別互相覆蓋（00730）。
#[test]
fn monthly_periods_are_parsed_with_two_digit_months() {
    let html = wrap_rows(&[
        dividend_row("2026M9", "0.11", "-", "2026/10/19", "-", "2026/11/12", "-"),
        dividend_row("2026M10", "0.12", "-", "2026/11/18", "-", "2026/12/12", "-"),
        dividend_row("2025M12", "0.11", "-", "2026/01/20", "-", "2026/02/12", "-"),
    ]);
    let result = parse_dividend_html("00730", "https://example.test", &html).expect("解析成功");

    assert_eq!(
        periods(&result, 2026),
        vec![
            ("M09".to_string(), dec!(0.11), "2026-10-19".to_string()),
            ("M10".to_string(), dec!(0.12), "2026-11-18".to_string()),
            ("M12".to_string(), dec!(0.11), "2026-01-20".to_string()),
        ]
    );
    let december = result
        .get_dividend_by_year(2026)
        .unwrap()
        .iter()
        .find(|d| d.quarter == "M12")
        .unwrap();
    assert_eq!(december.year_of_dividend, 2025);
}

/// ETF 一季標三次同一個季別（00777B）時，依除息日先後拆成該季的三個月別。
#[test]
fn repeated_quarter_of_an_etf_becomes_monthly_periods() {
    let html = wrap_rows(&[
        dividend_row("2026Q2", "0.14", "-", "2026/09/16", "-", "2026/10/14", "-"),
        dividend_row("2026Q2", "0.14", "-", "2026/08/18", "-", "2026/09/14", "-"),
        dividend_row("2026Q2", "0.14", "-", "2026/07/16", "-", "2026/08/12", "-"),
    ]);
    let result = parse_dividend_html("00777B", "https://example.test", &html).expect("解析成功");

    assert_eq!(
        periods(&result, 2026),
        vec![
            ("M04".to_string(), dec!(0.14), "2026-07-16".to_string()),
            ("M05".to_string(), dec!(0.14), "2026-08-18".to_string()),
            ("M06".to_string(), dec!(0.14), "2026-09-16".to_string()),
        ]
    );
}

/// ETF 一年兩次都標 H1（006208）時，依除息日先後拆成 H1、H2，兩次配息都保留。
#[test]
fn repeated_half_year_of_an_etf_becomes_h1_and_h2() {
    let html = wrap_rows(&[
        dividend_row("2025H1", "3.448", "-", "2025/11/18", "-", "2025/12/10", "-"),
        dividend_row("2025H1", "0.989", "-", "2025/07/16", "-", "2025/08/08", "-"),
    ]);
    let result = parse_dividend_html("006208", "https://example.test", &html).expect("解析成功");

    assert_eq!(
        periods(&result, 2025),
        vec![
            ("H1".to_string(), dec!(0.989), "2025-07-16".to_string()),
            ("H2".to_string(), dec!(3.448), "2025-11-18".to_string()),
        ]
    );
}

/// 完全相同的兩列只留一列（00758B 2023Q4）。
#[test]
fn identical_rows_are_deduplicated() {
    let row = dividend_row("2022Q4", "0.57", "-", "2023/02/23", "-", "2023/03/20", "-");
    let html = wrap_rows(&[row.clone(), row]);
    let result = parse_dividend_html("00758B", "https://example.test", &html).expect("解析成功");

    assert_eq!(
        periods(&result, 2023),
        vec![("Q4".to_string(), dec!(0.57), "2023-02-23".to_string())]
    );
}

/// 個股同一次配息被列兩次（延後除息前的舊日期）時，只留除息日最晚的一列（1109 2022 年）。
#[test]
fn repeated_period_of_a_stock_keeps_the_latest_ex_date() {
    let html = wrap_rows(&[
        dividend_row("2021", "1.5035", "-", "2022/08/04", "-", "2022/09/01", "-"),
        dividend_row("2021", "1.50", "-", "2022/07/14", "-", "2022/08/10", "-"),
    ]);
    let result = parse_dividend_html("1109", "https://example.test", &html).expect("解析成功");

    assert_eq!(
        periods(&result, 2022),
        vec![("".to_string(), dec!(1.5035), "2022-08-04".to_string())]
    );
}

/// ETF 重複得無法判斷怎麼拆（例如同一季四次）時，保留除息日最晚的一列。
#[test]
fn unsplittable_repeats_keep_the_latest_row() {
    let html = wrap_rows(&[
        dividend_row("2026Q1", "0.1", "-", "2026/01/16", "-", "-", "-"),
        dividend_row("2026Q1", "0.1", "-", "2026/02/16", "-", "-", "-"),
        dividend_row("2026Q1", "0.1", "-", "2026/03/16", "-", "-", "-"),
        dividend_row("2026Q1", "0.2", "-", "2026/03/30", "-", "-", "-"),
    ]);
    let result = parse_dividend_html("00999", "https://example.test", &html).expect("解析成功");

    assert_eq!(
        periods(&result, 2026),
        vec![("Q1".to_string(), dec!(0.2), "2026-03-30".to_string())]
    );
}

/// ETF 期別只有年份、一年配好幾次（00930 雙月配）時，以除息月份標記，每一次都保留。
#[test]
fn repeated_year_only_periods_of_an_etf_use_the_ex_date_month() {
    let html = wrap_rows(&[
        dividend_row("2026", "0.815", "-", "2026/09/23", "-", "2026/10/20", "-"),
        dividend_row("2026", "0.25", "-", "2026/07/27", "-", "2026/08/20", "-"),
        dividend_row("2026", "0.31", "-", "2026/05/26", "-", "2026/06/20", "-"),
        dividend_row("2026", "0.12", "-", "尚未公布", "-", "尚未公布", "-"),
        dividend_row("2025", "0.141", "-", "2026/01/20", "-", "2026/02/12", "-"),
    ]);
    let result = parse_dividend_html("00930", "https://example.test", &html).expect("解析成功");

    assert_eq!(
        periods(&result, 2026),
        vec![
            ("A".to_string(), dec!(0.141), "2026-01-20".to_string()),
            ("M05".to_string(), dec!(0.31), "2026-05-26".to_string()),
            ("M07".to_string(), dec!(0.25), "2026-07-27".to_string()),
            ("M09".to_string(), dec!(0.815), "2026-09-23".to_string()),
        ]
    );
}

/// 同一個月份除息兩次時無法以月份區分，保留日期最晚的一列。
#[test]
fn year_only_repeats_in_the_same_month_keep_the_latest_row() {
    let html = wrap_rows(&[
        dividend_row("2026", "0.1", "-", "2026/03/02", "-", "-", "-"),
        dividend_row("2026", "0.2", "-", "2026/03/30", "-", "-", "-"),
    ]);
    let result = parse_dividend_html("00999", "https://example.test", &html).expect("解析成功");

    assert_eq!(
        periods(&result, 2026),
        vec![("".to_string(), dec!(0.2), "2026-03-30".to_string())]
    );
}
