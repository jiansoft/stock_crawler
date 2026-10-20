//! 比對規則（同日、日期延後、日期未公布）的測試。

use super::*;

/// 同一天兩次配息時，交易所息值是合計：兩列都算已收錄，金額不動（2505 國揚 2020-08-18）。
#[test]
fn two_rows_on_the_same_day_are_matched_without_amount_changes() {
    let rows = existing(vec![
        row(1, 2020, "A", dec!(0.15), Decimal::ZERO, "2020-08-18", "-"),
        row(2, 2020, "Q2", dec!(1.5), Decimal::ZERO, "2020-08-18", "-"),
    ]);
    let plan = build_plan(&[cash_event("2020-08-18", dec!(1.65))], &rows, AS_OF);

    assert_eq!(plan.matched, 1);
    assert!(plan.updates.is_empty());
    assert!(plan.missing.is_empty());
}

/// 截止日之後的日期是尚未發生的事件，不可以被當成舊日期搬走。
#[test]
fn future_dates_are_not_treated_as_stale() {
    let rows = existing(vec![row(
        1,
        2026,
        "",
        dec!(0.11),
        Decimal::ZERO,
        "2026-10-19",
        "-",
    )]);
    let plan = build_plan(&[cash_event("2026-09-16", dec!(0.11))], &rows, AS_OF);

    assert!(plan.updates.is_empty());
    assert_eq!(plan.missing.len(), 1);
}

/// ETF 配息間隔短，不做日期搬移（只做同日與未公布日期的比對）。
#[test]
fn etf_dates_are_never_shifted() {
    let mut stale = row(1, 2025, "M05", dec!(0.11), Decimal::ZERO, "2025-06-01", "-");
    stale.security_code = "00730".to_string();
    let rows = HashMap::from([("00730".to_string(), vec![stale])]);
    let event = ExDividendAnnouncement {
        stock_symbol: "00730".to_string(),
        ..cash_event("2025-06-17", dec!(0.11))
    };
    let plan = build_plan(&[event], &rows, AS_OF);

    assert!(plan.updates.is_empty());
    assert_eq!(plan.missing.len(), 1);
}

/// 同日已收錄：只把四捨五入過的金額換成交易所金額。
#[test]
fn same_day_event_only_corrects_the_amount() {
    let rows = existing(vec![row(
        1,
        2022,
        "",
        dec!(1.5),
        Decimal::ZERO,
        "2022-08-04",
        "-",
    )]);
    let plan = build_plan(&[cash_event("2022-08-04", dec!(1.5035))], &rows, AS_OF);

    assert_eq!(plan.matched, 1);
    let updated = plan.updates.get(&1).expect("金額應修正");
    assert_eq!(updated.cash_dividend, dec!(1.5035));
    assert_eq!(updated.sum, dec!(1.5035));
    assert!(plan.missing.is_empty());
}

/// 金額一樣、日期也一樣時不產生異動。
#[test]
fn identical_event_is_left_untouched() {
    let rows = existing(vec![row(
        1,
        2022,
        "",
        dec!(1.5035),
        Decimal::ZERO,
        "2022-08-04",
        "-",
    )]);
    let plan = build_plan(&[cash_event("2022-08-04", dec!(1.5035))], &rows, AS_OF);
    assert_eq!(plan.matched, 1);
    assert!(plan.updates.is_empty());
}

/// 延後除息前的舊日期（07-14）改成交易所日期（08-04）。
#[test]
fn stale_date_is_moved_to_the_official_date() {
    let rows = existing(vec![row(
        1,
        2022,
        "",
        dec!(1.5),
        Decimal::ZERO,
        "2022-07-14",
        "-",
    )]);
    let plan = build_plan(&[cash_event("2022-08-04", dec!(1.5035))], &rows, AS_OF);

    let updated = plan.updates.get(&1).expect("日期應改寫");
    assert_eq!(updated.ex_dividend_date_cash, "2022-08-04");
    assert_eq!(updated.cash_dividend, dec!(1.5035));
}

/// 一年配兩次但資料庫只有第一次：第二次不可以被配到第一次的資料列上（006208 的 11 月）。
#[test]
fn a_row_already_matching_an_official_date_is_not_moved() {
    let rows = existing(vec![row(
        1,
        2025,
        "H1",
        dec!(0.989),
        Decimal::ZERO,
        "2025-07-16",
        "-",
    )]);
    let plan = build_plan(
        &[
            cash_event("2025-07-16", dec!(0.989)),
            cash_event("2025-11-18", dec!(3.448)),
        ],
        &rows,
        AS_OF,
    );

    assert_eq!(plan.matched, 1);
    assert!(plan.updates.is_empty());
    assert_eq!(plan.missing.len(), 1);
    assert_eq!(
        plan.missing[0].ex_date,
        NaiveDate::from_ymd_opt(2025, 11, 18).unwrap()
    );
}

/// 日期還沒公布、金額相同的列補上日期。
#[test]
fn unannounced_row_gets_the_official_date() {
    let rows = existing(vec![row(
        1,
        2023,
        "H2",
        dec!(0.5),
        Decimal::ZERO,
        "尚未公布",
        "尚未公布",
    )]);
    let plan = build_plan(&[cash_event("2023-07-11", dec!(0.5))], &rows, AS_OF);

    let updated = plan.updates.get(&1).expect("應補上日期");
    assert_eq!(updated.ex_dividend_date_cash, "2023-07-11");
}

/// 兩列都可能是同一次配息時不猜，列為無法判斷。
#[test]
fn two_candidate_rows_are_ambiguous() {
    let rows = existing(vec![
        row(1, 2023, "H1", dec!(0.5), Decimal::ZERO, "尚未公布", "-"),
        row(2, 2023, "H2", dec!(0.5), Decimal::ZERO, "尚未公布", "-"),
    ]);
    let plan = build_plan(&[cash_event("2023-07-11", dec!(0.5))], &rows, AS_OF);

    assert!(plan.updates.is_empty());
    assert_eq!(plan.ambiguous.len(), 1);
}

/// 金額差太多不算同一次配息；年度合計列不參與比對。
#[test]
fn far_amounts_and_annual_totals_are_not_matched() {
    let rows = existing(vec![
        row(1, 2023, "", dec!(2.2), Decimal::ZERO, "-", "-"),
        row(2, 2023, "H1", dec!(5.0), Decimal::ZERO, "2023-06-01", "-"),
        row(3, 2023, "H2", dec!(1.6), Decimal::ZERO, "2023-11-29", "-"),
    ]);
    let plan = build_plan(&[cash_event("2023-06-19", dec!(1.6))], &rows, AS_OF);

    assert!(plan.updates.is_empty());
    assert_eq!(plan.missing.len(), 1);
}

/// 證交所單純「權」沒有金額、也可能是現金增資：對不上時不列入缺漏。
#[test]
fn unmatched_rights_only_event_is_not_reported() {
    let mut event = cash_event("2025-01-06", Decimal::ZERO);
    event.is_cash = false;
    event.is_stock = true;
    event.cash_dividend = None;
    let plan = build_plan(&[event], &existing(vec![]), AS_OF);
    assert!(plan.missing.is_empty());
}

/// 櫃買有拆分金額：同日除權息時一併修正股票股利（4175 的 1.0 → 0.9632）。
#[test]
fn combined_event_corrects_the_stock_amount() {
    let rows = existing(vec![row(
        1,
        2023,
        "",
        dec!(2.7452),
        dec!(1.0),
        "2023-07-20",
        "2023-07-21",
    )]);
    let event = ExDividendAnnouncement {
        is_stock: true,
        stock_dividend_ratio: Some(dec!(0.09632195232)),
        market: StockExchangeMarket::OverTheCounter,
        ..cash_event("2023-07-20", dec!(2.7452))
    };
    let plan = build_plan(&[event], &rows, AS_OF);

    let updated = plan.updates.get(&1).expect("股票股利應修正");
    assert_eq!(updated.stock_dividend, dec!(0.9632));
    assert_eq!(updated.sum, dec!(3.7084));
}

/// 同日除權息、資料列缺另一欄日期時補上。
#[test]
fn combined_event_fills_the_missing_date() {
    let rows = existing(vec![row(
        1,
        2023,
        "",
        dec!(0.2),
        dec!(0.4),
        "2024-08-26",
        "-",
    )]);
    let event = ExDividendAnnouncement {
        is_stock: true,
        stock_dividend_ratio: Some(dec!(0.04)),
        ..cash_event("2024-08-26", dec!(0.2))
    };
    let plan = build_plan(&[event], &rows, AS_OF);

    let updated = plan.updates.get(&1).expect("除權日應補上");
    assert_eq!(updated.ex_dividend_date_stock, "2024-08-26");
}

fn stock_event(date: &str, ratio: Decimal) -> ExDividendAnnouncement {
    ExDividendAnnouncement {
        is_cash: false,
        is_stock: true,
        cash_dividend: None,
        stock_dividend_ratio: Some(ratio),
        market: StockExchangeMarket::OverTheCounter,
        ..cash_event(date, Decimal::ZERO)
    }
}

/// 年度股利的除息、除權不同天（4549 桓達 2022 年 06-28 除息、08-24 除權）：
/// 兩次交易所事件分別對上同一列的現金側與股票側，不可把除權事件判成缺漏。
#[test]
fn cash_and_stock_events_on_different_days_match_the_same_row() {
    let rows = existing(vec![row(
        1,
        2022,
        "",
        dec!(4.1),
        dec!(1.1),
        "2022-06-28",
        "2022-08-24",
    )]);
    let plan = build_plan(
        &[
            cash_event("2022-06-28", dec!(4.1)),
            stock_event("2022-08-24", dec!(0.11)),
        ],
        &rows,
        AS_OF,
    );

    assert_eq!(plan.matched, 2);
    assert!(plan.missing.is_empty());
    assert!(plan.updates.is_empty());
    assert_eq!(plan.used, HashSet::from([1]));
}

/// 股票側日期未公布時，除息先對上現金側，除權仍能以「日期未公布」補上股票側日期。
#[test]
fn stock_event_fills_the_unannounced_stock_date_of_a_matched_row() {
    let rows = existing(vec![row(
        1,
        2024,
        "",
        dec!(0.8),
        dec!(0.8),
        "2024-03-28",
        "-",
    )]);
    let plan = build_plan(
        &[
            cash_event("2024-03-28", dec!(0.8)),
            stock_event("2024-09-05", dec!(0.08)),
        ],
        &rows,
        AS_OF,
    );

    assert!(plan.missing.is_empty());
    assert_eq!(plan.updates[&1].ex_dividend_date_stock, "2024-09-05");
    assert_eq!(plan.updates[&1].ex_dividend_date_cash, "2024-03-28");
}
