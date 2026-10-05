//! ex_right_reconcile 的單元測試。

use chrono::Local;
use rust_decimal_macros::dec;

use super::plan::*;
use super::yahoo_fill::*;
use super::*;
use crate::core::declare::StockExchangeMarket;
use rust_decimal::Decimal;

fn row(
    serial: i64,
    year: i32,
    quarter: &str,
    cash: Decimal,
    stock: Decimal,
    ex_cash: &str,
    ex_stock: &str,
) -> Dividend {
    Dividend {
        serial,
        year,
        year_of_dividend: year - 1,
        quarter: quarter.to_string(),
        security_code: "1109".to_string(),
        earnings_cash_dividend: Decimal::ZERO,
        capital_reserve_cash_dividend: Decimal::ZERO,
        cash_dividend: cash,
        earnings_stock_dividend: Decimal::ZERO,
        capital_reserve_stock_dividend: Decimal::ZERO,
        stock_dividend: stock,
        sum: cash + stock,
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: ex_cash.to_string(),
        ex_dividend_date_stock: ex_stock.to_string(),
        payable_date_cash: "-".to_string(),
        payable_date_stock: "-".to_string(),
        created_time: Local::now(),
        updated_time: Local::now(),
    }
}

fn cash_event(date: &str, cash: Decimal) -> ExDividendAnnouncement {
    ExDividendAnnouncement {
        stock_symbol: "1109".to_string(),
        name: "信大".to_string(),
        ex_date: NaiveDate::parse_from_str(date, DATE_FORMAT).unwrap(),
        is_cash: true,
        is_stock: false,
        cash_dividend: Some(cash),
        stock_dividend_ratio: None,
        market: StockExchangeMarket::Listed,
    }
}

fn existing(rows: Vec<Dividend>) -> HashMap<String, Vec<Dividend>> {
    HashMap::from([("1109".to_string(), rows)])
}

/// 測試用的交易所資料截止日。
const AS_OF: NaiveDate = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();

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

#[test]
fn calendar_year_chunks_split_on_year_boundaries() {
    let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
    assert_eq!(
        calendar_year_chunks(d(2024, 11, 1), d(2026, 2, 3)),
        vec![
            (d(2024, 11, 1), d(2024, 12, 31)),
            (d(2025, 1, 1), d(2025, 12, 31)),
            (d(2026, 1, 1), d(2026, 2, 3)),
        ]
    );
    assert!(calendar_year_chunks(d(2026, 2, 3), d(2026, 1, 1)).is_empty());
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

    assert_eq!(
        unresolved_after_fill(&[&filled, &unfilled, &stock_event], &fills),
        vec!["1109 2026-10-28".to_string()]
    );
    assert_eq!(
        unresolved_after_fill(&[&filled], &[]),
        vec!["1109 2026-09-23".to_string()]
    );
    assert!(unresolved_after_fill(&[], &fills).is_empty());
}

/// 日誌只列前 20 筆，沒有缺漏時不記錄。
#[test]
fn missing_events_are_labelled_and_warned() {
    assert_eq!(
        event_label(&cash_event("2026-09-23", dec!(0.08))),
        "1109 2026-09-23"
    );
    warn_missing_events("測試", &[]);
    let many: Vec<String> = (0..25).map(|index| format!("1109 #{index}")).collect();
    warn_missing_events("測試", &many);
}
