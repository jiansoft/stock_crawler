//! 真實頁面：年度配股併入半年配事件、年度與半年配混合時保留事件並略過合計。

use super::*;

/// 以 2753 的實際頁面驗證整年度配股會併回 25H2，且擬定列不會蓋掉已公布的配息。
///
/// Yahoo 把 2753 的 0.5 元配股拆成獨立的「2025」列，另外又在表格最上方留了一筆
/// 日期未定的 2025H2 配股；Goodinfo 的除權息日程則是把 7.5 元現金與 0.5 元配股
/// 都掛在 25H2。這個測試釘住「與 Goodinfo 一致」的解析結果。
#[test]
fn real_page_merges_annual_stock_dividend_into_half_year_event() {
    let html = include_str!("../../testdata/dividend_2753.html");
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
