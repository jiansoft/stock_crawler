use rust_decimal_macros::dec;

use super::*;

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
}

fn payout(payable: NaiveDate, cash: Decimal, stock_shares: i64, estimated: bool) -> DividendPayout {
    DividendPayout {
        member_id: 0,
        payable_date: payable,
        stock_symbol: "0056".to_owned(),
        name: "元大高股息".to_owned(),
        quarter: "Q3".to_owned(),
        ex_date: None,
        shares: 1000,
        cash_dividend: dec!(1.07),
        stock_dividend: Decimal::ZERO,
        cash_amount: cash,
        stock_shares,
        estimated,
    }
}

/// 今年依月份加總並附去年同月；去年同期只算到去年的同一天。
#[test]
fn build_response_sums_months_and_compares_last_year() {
    let as_of = ymd(2026, 11, 3);
    let this_year = [
        payout(ymd(2026, 1, 15), dec!(1000.006), 0, false),
        payout(ymd(2026, 11, 20), dec!(500), 30, false),
        payout(ymd(2026, 12, 10), dec!(200), 0, true),
    ];
    let last_year = [
        payout(ymd(2025, 1, 15), dec!(800), 0, false),
        payout(ymd(2025, 11, 20), dec!(400), 0, false),
    ];
    let response = build_response(7, 2026, as_of, &this_year, &last_year, false);

    assert_eq!((response.member_id, response.year), (7, 2026));
    assert_eq!(response.as_of, "2026-11-03");
    assert_eq!(response.total_cash_amount, 1700.01);
    assert_eq!(response.received_cash_amount, 1000.01);
    assert_eq!(response.estimated_cash_amount, 200.0);
    assert_eq!(response.total_stock_shares, 30);
    assert_eq!(response.last_year_cash_amount, 1200.0);
    assert_eq!(
        response.last_year_same_period_cash_amount, 800.0,
        "去年 11-20 晚於去年的 11-03，不算同期"
    );
    assert_eq!(response.months.len(), 12);
    assert_eq!(
        response.months[10],
        DividendIncomeMonth {
            month: 11,
            cash_amount: 500.0,
            received_cash_amount: 0.0,
            estimated_cash_amount: 0.0,
            stock_shares: 30,
            payouts: 1,
            last_year_cash_amount: 400.0,
        }
    );
    assert_eq!(response.months[11].estimated_cash_amount, 200.0);
    assert_eq!(response.months[5].cash_amount, 0.0);
    assert_eq!(response.items, None, "沒帶 include_items 不回明細");

    let detailed = build_response(7, 2026, as_of, &this_year, &last_year, true);
    let items = detailed.items.expect("帶 include_items 要回明細");
    let summary: Vec<(&str, f64, i64, bool)> = items
        .iter()
        .map(|item| {
            (
                item.payable_date.as_str(),
                item.cash_amount,
                item.stock_shares,
                item.estimated,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("2026-01-15", 1000.01, 0, false),
            ("2026-11-20", 500.0, 30, false),
            ("2026-12-10", 200.0, 0, true),
        ]
    );
    assert_eq!(
        (items[0].stock_symbol.as_str(), items[0].quarter.as_str()),
        ("0056", "Q3")
    );
}

/// 已過完的年度：去年同期就是去年全年。
#[test]
fn build_response_for_past_year_compares_whole_last_year() {
    let last_year = [payout(ymd(2024, 12, 20), dec!(300), 0, false)];
    let response = build_response(0, 2025, ymd(2026, 11, 3), &[], &last_year, true);
    assert_eq!(response.total_cash_amount, 0.0);
    assert_eq!(response.last_year_cash_amount, 300.0);
    assert_eq!(response.last_year_same_period_cash_amount, 300.0);
    assert_eq!(
        response.items,
        Some(Vec::new()),
        "今年沒有入帳時明細為空清單"
    );
}

#[test]
fn year_range_accepts_2000_through_next_year() {
    let today = ymd(2026, 11, 3);
    assert_eq!(
        year_range(2026, today),
        Some((ymd(2026, 1, 1), ymd(2026, 12, 31)))
    );
    assert!(year_range(2000, today).is_some());
    assert!(year_range(2027, today).is_some());
    assert_eq!(year_range(1999, today), None);
    assert_eq!(year_range(2028, today), None);
}
