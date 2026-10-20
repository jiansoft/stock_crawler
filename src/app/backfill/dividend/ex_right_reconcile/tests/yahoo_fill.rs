//! 從 Yahoo 補缺漏事件與配股金額的測試。

use super::*;

/// 缺漏事件從 Yahoo 補：挑同一天除權息的明細，取 Yahoo 的所屬年度與期別。
#[test]
fn select_fills_takes_the_yahoo_detail_of_the_same_day() {
    let mut second_h2 = row(
        10,
        2024,
        "H2",
        dec!(0.4863),
        dec!(0.214),
        "2024-09-25",
        "2024-09-25",
    );
    second_h2.year_of_dividend = 2023;
    let unrelated = row(11, 2024, "H1", dec!(0.1), Decimal::ZERO, "2024-03-01", "-");
    let event = cash_event("2024-09-25", dec!(0.4863));

    let chosen = select_fills(&[&event], &[unrelated, second_h2], &[], &HashSet::new());
    assert_eq!(chosen.len(), 1);
    assert_eq!(chosen[0].quarter, "H2");
    assert_eq!(chosen[0].year_of_dividend, 2023);
}

/// 目標鍵上已有一列且已對上交易所事件時不覆蓋；同一筆 Yahoo 明細也不會補兩次。
#[test]
fn select_fills_never_overwrites_a_confirmed_row() {
    let confirmed = row(
        1,
        2024,
        "H2",
        dec!(0.0919),
        Decimal::ZERO,
        "2023-12-12",
        "-",
    );
    let mut yahoo_row = row(10, 2024, "H2", dec!(0.5), Decimal::ZERO, "2024-09-25", "-");
    yahoo_row.year_of_dividend = confirmed.year_of_dividend;
    let event = cash_event("2024-09-25", dec!(0.5));
    let used = HashSet::from([1]);

    assert!(
        select_fills(
            &[&event],
            std::slice::from_ref(&yahoo_row),
            std::slice::from_ref(&confirmed),
            &used
        )
        .is_empty()
    );
    // 那列沒有對上任何交易所事件（是錯的或舊的）時可以覆蓋。
    assert_eq!(
        select_fills(
            &[&event],
            &[yahoo_row.clone()],
            &[confirmed],
            &HashSet::new()
        )
        .len(),
        1
    );
    // 兩個缺漏事件指到同一筆明細時只補一次。
    let twice = select_fills(&[&event, &event], &[yahoo_row], &[], &HashSet::new());
    assert_eq!(twice.len(), 1);
}

/// Yahoo 也找不到同一天的明細時不補。
#[test]
fn select_fills_skips_events_without_a_yahoo_match() {
    let yahoo_row = row(10, 2024, "H2", dec!(0.5), Decimal::ZERO, "2024-09-26", "-");
    let event = cash_event("2024-09-25", dec!(0.5));
    assert!(select_fills(&[&event], &[yahoo_row], &[], &HashSet::new()).is_empty());
}

/// 證交所「權息」對上了只有現金的列：記下來，再用 Yahoo 同一除權日的配股補上（2327 國巨）。
#[test]
fn missing_stock_amount_is_filled_from_yahoo() {
    let rows = existing(vec![row(
        1,
        2024,
        "",
        dec!(2.0),
        Decimal::ZERO,
        "2024-08-15",
        "-",
    )]);
    let event = ExDividendAnnouncement {
        is_stock: true,
        cash_dividend: None,
        ..cash_event("2024-08-15", Decimal::ZERO)
    };
    let plan = build_plan(std::slice::from_ref(&event), &rows, AS_OF);
    assert_eq!(plan.stock_unknown.len(), 1);
    assert_eq!(plan.stock_unknown[0].0, 1);

    let yahoo = row(
        10,
        2024,
        "",
        dec!(2.0),
        dec!(1.9484),
        "2024-08-15",
        "2024-08-15",
    );
    let fixed = stock_from_yahoo(&rows["1109"][0], &event, &[yahoo]).expect("應補上配股");
    assert_eq!(fixed.stock_dividend, dec!(1.9484));
    assert_eq!(fixed.sum, dec!(3.9484));
    assert_eq!(fixed.ex_dividend_date_stock, "2024-08-15");
    assert_eq!(fixed.serial, 1);

    // Yahoo 也沒有配股時不動。
    let cash_only = row(10, 2024, "", dec!(2.0), Decimal::ZERO, "2024-08-15", "-");
    assert!(stock_from_yahoo(&rows["1109"][0], &event, &[cash_only]).is_none());
}

/// 補入列與事件同一天才算補上；Yahoo 沒有同日資料的事件列為補不到
/// （00990B 2026-09-23 被同一輪補上，就不該再列為缺漏）。
#[test]
fn unresolved_after_fill_lists_only_events_without_a_fill() {
    let filled = cash_event("2026-09-23", dec!(0.08));
    let unfilled = cash_event("2026-10-28", dec!(0.08));
    let stock_fill = row(11, 2026, "Q2", Decimal::ZERO, dec!(0.5), "-", "2026-07-15");
    let stock_event = cash_event("2026-07-15", dec!(0.5));
    let fills = vec![
        row(
            10,
            2026,
            "M08",
            dec!(0.08),
            Decimal::ZERO,
            "2026-09-23",
            "-",
        ),
        stock_fill,
    ];

    let labels = |events: Vec<ExDividendAnnouncement>| -> Vec<String> {
        events.iter().map(event_label).collect()
    };
    assert_eq!(
        labels(unresolved_after_fill(
            &[&filled, &unfilled, &stock_event],
            &fills
        )),
        vec!["1109 2026-10-28".to_string()]
    );
    assert_eq!(
        labels(unresolved_after_fill(&[&filled], &[])),
        vec!["1109 2026-09-23".to_string()]
    );
    assert!(unresolved_after_fill(&[], &fills).is_empty());
}
