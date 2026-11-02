use rust_decimal_macros::dec;

use super::*;

fn date(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).expect("合法日期")
}

/// 入帳逐筆轉成 JSON：每股股利取 4 位、金額取 2 位小數；合計只算現金。
#[test]
fn build_response_maps_payouts_and_total() {
    let payouts = [
        DividendPayout {
            member_id: 0,
            payable_date: date(11, 15),
            stock_symbol: "1101".to_owned(),
            name: "台泥".to_owned(),
            quarter: "Q3".to_owned(),
            ex_date: Some(date(11, 10)),
            shares: 1000,
            cash_dividend: dec!(2.123456),
            stock_dividend: Decimal::ZERO,
            cash_amount: dec!(2123.456),
            stock_shares: 0,
            estimated: false,
        },
        DividendPayout {
            member_id: 0,
            payable_date: date(11, 18),
            stock_symbol: "2002".to_owned(),
            name: "中鋼".to_owned(),
            quarter: String::new(),
            ex_date: None,
            shares: 2000,
            cash_dividend: Decimal::ZERO,
            stock_dividend: dec!(0.5),
            cash_amount: Decimal::ZERO,
            stock_shares: 100,
            estimated: true,
        },
    ];
    let response = build_response(7, date(11, 2), date(12, 1), &payouts);

    assert_eq!(response.member_id, 7);
    assert_eq!(
        (response.from.as_str(), response.to.as_str()),
        ("2026-11-02", "2026-12-01")
    );
    assert_eq!(response.total_cash_amount, 2123.46);
    assert_eq!(response.items.len(), 2);
    let first = &response.items[0];
    assert_eq!(
        (
            first.payable_date.as_str(),
            first.ex_date.as_deref(),
            first.cash_dividend,
            first.cash_amount
        ),
        ("2026-11-15", Some("2026-11-10"), 2.1235, 2123.46)
    );
    let second = &response.items[1];
    assert_eq!(
        (
            second.ex_date.as_deref(),
            second.stock_dividend,
            second.stock_shares,
            second.estimated
        ),
        (None, 0.5, 100, true)
    );
}
