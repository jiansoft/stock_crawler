//! 依除息日持股計算應得配息：多會員合併、略過推估列。

use super::*;
use crate::domain::portfolio::payout::PayoutAllocation;

#[allow(clippy::too_many_arguments)]
fn allocation(
    member_id: i64,
    symbol: &str,
    quarter: &str,
    cash: Decimal,
    ex_date: NaiveDate,
    pay_date: NaiveDate,
    shares: i64,
    estimated: bool,
) -> PayoutAllocation {
    PayoutAllocation {
        member_id,
        stock_symbol: symbol.to_owned(),
        name: format!("{symbol} 名稱"),
        quarter: quarter.to_owned(),
        cash_dividend: cash,
        stock_dividend: Decimal::ZERO,
        cash_payable: Some(pay_date),
        stock_payable: None,
        cash_ex: Some(ex_date),
        stock_ex: None,
        shares,
        cash_amount: cash * Decimal::from(shares),
        stock_shares: Decimal::ZERO,
        estimated,
    }
}

/// 各會員合併；以目前持股推算、沒有現金的不取；同除息日的季別與月別列取金額大者。
#[test]
fn entitlements_merge_members_and_skip_estimates() {
    let (ex, pay) = (date(2026, 10, 22), date(2026, 11, 11));
    let allocations = [
        allocation(1, "0056", "Q3", dec!(1.72), ex, pay, 1_000, false),
        allocation(2, "0056", "Q3", dec!(1.72), ex, pay, 670, false),
        // 同除息日的月別列金額較小 → 去重時不取。
        allocation(1, "0056", "M10", dec!(0.5), ex, pay, 1_000, false),
        // 尚未除息、以目前持股推算 → 不取。
        allocation(
            1,
            "2330",
            "Q3",
            dec!(5),
            date(2026, 12, 10),
            date(2027, 1, 8),
            100,
            true,
        ),
        // 只有股票股利 → 不取。
        allocation(1, "2884", "", Decimal::ZERO, ex, pay, 100, false),
    ];

    let rows = entitlements(&allocations);

    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        (rows[0].shares, rows[0].amount, rows[0].cash_per_share),
        (1_670, dec!(2872.40), dec!(1.72))
    );
}

/// 已除息未發放的已公告配息改用除息日持股；持股已賣光但已除息的仍列入；未除息的照目前持股。
#[test]
fn entitled_payouts_use_ex_date_shares() {
    let today = date(2026, 11, 3);
    let events = [
        event(
            "0056",
            2026,
            "Q3",
            dec!(1.72),
            Some(date(2026, 10, 22)),
            Some(date(2026, 11, 11)),
        ),
        event(
            "0056",
            2026,
            "Q4",
            dec!(1),
            Some(date(2027, 1, 21)),
            Some(date(2027, 2, 10)),
        ),
    ];
    let records = [
        Entitlement {
            stock_symbol: "0056".to_owned(),
            name: "0056 名稱".to_owned(),
            ex_date: date(2026, 10, 22),
            pay_date: date(2026, 11, 11),
            cash_per_share: dec!(1.72),
            shares: 1_670,
            amount: dec!(2872.4),
        },
        // 除息後已全部賣出，不在目前持股。
        Entitlement {
            stock_symbol: "2884".to_owned(),
            name: "2884 名稱".to_owned(),
            ex_date: date(2026, 10, 30),
            pay_date: date(2026, 11, 20),
            cash_per_share: dec!(0.5),
            shares: 2_000,
            amount: dec!(1000),
        },
        // 發放日已過 → 不在預估期間。
        Entitlement {
            stock_symbol: "2884".to_owned(),
            name: "2884 名稱".to_owned(),
            ex_date: date(2026, 7, 10),
            pay_date: date(2026, 8, 1),
            cash_per_share: dec!(0.5),
            shares: 2_000,
            amount: dec!(1000),
        },
    ];

    let forecast = forecast(&[holding("0056", 1_740, None)], &events, &records, today, 6);

    let summary: Vec<(&str, i64, Decimal, bool)> = forecast
        .items
        .iter()
        .map(|item| {
            (
                item.stock_symbol.as_str(),
                item.shares,
                item.amount,
                item.entitled,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("0056", 1_670, dec!(2872.4), true),
            ("2884", 2_000, dec!(1000), true),
            ("0056", 1_740, dec!(1740), false),
        ]
    );
    assert_eq!(forecast.announced_total, 5612.4);
}
