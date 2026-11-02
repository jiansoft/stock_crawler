use rust_decimal_macros::dec;

use super::*;

fn date(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).expect("合法日期")
}

fn row(
    symbol: &str,
    cash_payable: Option<NaiveDate>,
    stock_payable: Option<NaiveDate>,
) -> PayoutRow {
    PayoutRow {
        stock_symbol: symbol.to_owned(),
        name: format!("{symbol} 名稱"),
        quarter: "Q3".to_owned(),
        cash_dividend: dec!(2.5),
        stock_dividend: dec!(0.5),
        cash_payable,
        stock_payable,
        cash_ex: Some(date(11, 10)),
        stock_ex: Some(date(11, 11)),
        shares: 1000,
        cash_amount: dec!(2500),
        stock_shares: dec!(50.7),
        estimated: false,
    }
}

/// 現金與股票同一天發放合為一筆；不同天拆成兩筆，各帶自己的除權息日；期間外的那部分不列；
/// 依發放日、代號排序，合計只算期間內的現金。
#[test]
fn build_response_splits_by_payable_date() {
    let rows = [
        row("2330", Some(date(11, 20)), Some(date(11, 20))),
        row("1101", Some(date(11, 15)), Some(date(12, 3))),
        row("2002", Some(date(12, 31)), Some(date(11, 18))),
    ];
    let response = build_response(0, date(11, 2), date(12, 1), &rows);

    let summary: Vec<(&str, &str, Option<&str>, f64, i64)> = response
        .items
        .iter()
        .map(|item| {
            (
                item.payable_date.as_str(),
                item.stock_symbol.as_str(),
                item.ex_date.as_deref(),
                item.cash_amount,
                item.stock_shares,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("2026-11-15", "1101", Some("2026-11-10"), 2500.0, 0),
            ("2026-11-18", "2002", Some("2026-11-11"), 0.0, 50),
            ("2026-11-20", "2330", Some("2026-11-10"), 2500.0, 50),
        ]
    );
    assert_eq!(response.items[1].stock_dividend, 0.5);
    assert_eq!(
        response.items[1].cash_dividend, 0.0,
        "只發股票的那筆沒有現金"
    );
    assert_eq!(response.total_cash_amount, 5000.0, "2002 的現金在期間外");
    assert_eq!(
        (response.from.as_str(), response.to.as_str()),
        ("2026-11-02", "2026-12-01")
    );
}

/// 金額為 0 或不足一股的部分不列；沒有發放日的不列。
#[test]
fn build_response_skips_empty_parts() {
    let mut cash_only = row("2330", Some(date(11, 20)), Some(date(11, 20)));
    cash_only.stock_shares = dec!(0.4);
    let mut nothing = row("1101", None, Some(date(11, 20)));
    nothing.stock_shares = Decimal::ZERO;
    let response = build_response(7, date(11, 2), date(12, 1), &[cash_only, nothing]);

    assert_eq!(response.member_id, 7);
    assert_eq!(response.items.len(), 1);
    assert_eq!(response.items[0].stock_symbol, "2330");
    assert_eq!(response.items[0].stock_shares, 0);
    assert_eq!(
        response.items[0].stock_dividend, 0.0,
        "不足一股時不附每股股票股利"
    );
}
