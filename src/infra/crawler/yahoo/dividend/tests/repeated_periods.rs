//! 同一期別重複出現時的處理：ETF 月配、季配與半年配拆期、去重與保留最新一列。

use super::*;

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
