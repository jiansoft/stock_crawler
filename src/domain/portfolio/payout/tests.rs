use rust_decimal_macros::dec;

use super::*;

fn date(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).expect("合法日期")
}

fn allocation(
    symbol: &str,
    cash_payable: Option<NaiveDate>,
    stock_payable: Option<NaiveDate>,
) -> PayoutAllocation {
    PayoutAllocation {
        member_id: 1,
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
/// 依發放日、代號排序。
#[test]
fn split_by_payable_date_splits_cash_and_stock() {
    let allocations = [
        allocation("2330", Some(date(11, 20)), Some(date(11, 20))),
        allocation("1101", Some(date(11, 15)), Some(date(12, 3))),
        allocation("2002", Some(date(12, 31)), Some(date(11, 18))),
    ];
    let payouts = split_by_payable_date(&allocations, date(11, 2), date(12, 1));

    let summary: Vec<(NaiveDate, &str, Option<NaiveDate>, Decimal, i64)> = payouts
        .iter()
        .map(|p| {
            (
                p.payable_date,
                p.stock_symbol.as_str(),
                p.ex_date,
                p.cash_amount,
                p.stock_shares,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (date(11, 15), "1101", Some(date(11, 10)), dec!(2500), 0),
            (date(11, 18), "2002", Some(date(11, 11)), Decimal::ZERO, 50),
            (date(11, 20), "2330", Some(date(11, 10)), dec!(2500), 50),
        ]
    );
    assert_eq!(payouts[1].stock_dividend, dec!(0.5));
    assert_eq!(
        payouts[1].cash_dividend,
        Decimal::ZERO,
        "只發股票的那筆沒有現金"
    );
    assert_eq!(total_cash(&payouts), dec!(5000), "2002 的現金在期間外");
}

/// 金額為 0 或不足一股的部分不列；沒有發放日的不列。
#[test]
fn split_by_payable_date_skips_empty_parts() {
    let mut cash_only = allocation("2330", Some(date(11, 20)), Some(date(11, 20)));
    cash_only.stock_shares = dec!(0.4);
    let mut nothing = allocation("1101", None, Some(date(11, 20)));
    nothing.stock_shares = Decimal::ZERO;
    let payouts = split_by_payable_date(&[cash_only, nothing], date(11, 2), date(12, 1));

    assert_eq!(payouts.len(), 1);
    assert_eq!(payouts[0].stock_symbol, "2330");
    assert_eq!(payouts[0].stock_shares, 0);
    assert_eq!(
        payouts[0].stock_dividend,
        Decimal::ZERO,
        "不足一股時不附每股股票股利"
    );
}

/// 不同會員同一天同一筆股利加總成一筆；推算與已領的分開；其他筆照排序保留。
#[test]
fn merge_members_sums_the_same_payout() {
    let mut other_member = allocation("2330", Some(date(11, 20)), None);
    other_member.member_id = 2;
    other_member.shares = 500;
    other_member.cash_amount = dec!(1250);
    let mut estimated = allocation("2330", Some(date(11, 20)), None);
    estimated.member_id = 3;
    estimated.estimated = true;
    let allocations = [
        allocation("2330", Some(date(11, 20)), None),
        other_member,
        estimated,
        allocation("1101", Some(date(11, 15)), None),
    ];
    let merged = merge_members(split_by_payable_date(
        &allocations,
        date(11, 2),
        date(12, 1),
    ));

    let summary: Vec<(&str, i64, Decimal, bool, i64)> = merged
        .iter()
        .map(|p| {
            (
                p.stock_symbol.as_str(),
                p.shares,
                p.cash_amount,
                p.estimated,
                p.member_id,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("1101", 1000, dec!(2500), false, 0),
            ("2330", 1500, dec!(3750), false, 0),
            ("2330", 1000, dec!(2500), true, 0),
        ]
    );
}
