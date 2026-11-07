use rust_decimal_macros::dec;

use super::*;
use crate::domain::portfolio::payout::PayoutAllocation;

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("有效日期")
}

fn holding(symbol: &str, shares: i64, price: Option<Decimal>) -> Holding {
    Holding {
        stock_symbol: symbol.to_owned(),
        name: format!("{symbol} 名稱"),
        shares,
        closing_price: price,
    }
}

fn event(
    symbol: &str,
    year_of_dividend: i32,
    quarter: &str,
    cash: Decimal,
    ex_date: Option<NaiveDate>,
    payable_date: Option<NaiveDate>,
) -> DividendEvent {
    DividendEvent {
        stock_symbol: symbol.to_owned(),
        year_of_dividend,
        quarter: quarter.to_owned(),
        cash_per_share: cash,
        ex_date,
        payable_date,
    }
}

/// 已公告的配息直接列入；去年同期推一年、季配金額改用最近一期；已公告的同期不重複；
/// 全年彙總列不計。
#[test]
fn announced_and_estimated_without_double_counting() {
    let today = date(2026, 10, 12);
    let events = [
        // 已公告：10-22 除息、11-11 發放。
        event(
            "0056",
            2026,
            "Q3",
            dec!(1.72),
            Some(date(2026, 10, 22)),
            Some(date(2026, 11, 11)),
        ),
        // 去年同期（2025-10-23）推到 2026-10-23，與已公告的 10-22 同期 → 不重複。
        event(
            "0056",
            2025,
            "Q3",
            dec!(0.866),
            Some(date(2025, 10, 23)),
            Some(date(2025, 11, 14)),
        ),
        // 去年 Q4（2026-01-22）推到 2027-01-22、發放 2027-02-11，金額用最近一期 1.72。
        event(
            "0056",
            2025,
            "Q4",
            dec!(0.866),
            Some(date(2026, 1, 22)),
            Some(date(2026, 2, 11)),
        ),
        // 全年彙總列沒有日期、期別空白 → 不計。
        event("0056", 2025, "", dec!(4.936), None, None),
        // 已發放過的不列入已公告。
        event(
            "0056",
            2026,
            "Q2",
            dec!(1.35),
            Some(date(2026, 7, 21)),
            Some(date(2026, 8, 10)),
        ),
    ];
    let forecast = forecast(
        &[holding("0056", 1000, Some(dec!(40)))],
        &events,
        &[],
        today,
        12,
    );

    let summary: Vec<(NaiveDate, Decimal, Basis)> = forecast
        .items
        .iter()
        .map(|item| (item.pay_date, item.amount, item.basis))
        .collect();
    assert_eq!(
        summary,
        vec![
            (date(2026, 11, 11), dec!(1720), Basis::Announced),
            (date(2027, 2, 11), dec!(1720), Basis::LatestAmount),
            // 去年 Q2（2026-07-21）推到 2027-07-21、發放 2027-08-10。
            (date(2027, 8, 10), dec!(1720), Basis::LatestAmount),
        ]
    );
    assert_eq!(forecast.total, 5160.0);
    assert_eq!(
        (forecast.announced_total, forecast.estimated_total),
        (1720.0, 3440.0)
    );
    // 市值 40,000 → 12.9%。
    assert_eq!(forecast.forward_yield_percent, Some(12.9));
    assert_eq!(forecast.months.len(), 13);
    assert_eq!(forecast.months[0].month, "2026-10");
    let november = forecast
        .months
        .iter()
        .find(|m| m.month == "2026-11")
        .expect("11 月");
    assert_eq!((november.announced, november.estimated), (1720.0, 0.0));
}

/// 下一期金額已公告、除息日未定：用公告金額、日期依去年；沒有發放日以除息後 30 天推估。
#[test]
fn amount_announced_and_missing_payable_date() {
    let today = date(2026, 3, 1);
    let events = [
        event("2887", 2024, "", dec!(0.5), Some(date(2025, 7, 24)), None),
        event("2887", 2025, "", dec!(0.6), None, None),
    ];
    let forecast = forecast(&[holding("2887", 2000, None)], &events, &[], today, 12);
    assert_eq!(forecast.items.len(), 1);
    let item = &forecast.items[0];
    assert_eq!(item.basis, Basis::AmountAnnounced);
    assert_eq!(
        (item.ex_date, item.pay_date),
        (date(2026, 7, 24), date(2026, 8, 23))
    );
    assert_eq!(item.amount, dec!(1200));
    assert_eq!(forecast.announced_total, 1200.0);
    assert_eq!(forecast.forward_yield_percent, None);
}

/// 同一除息日的季別與月別兩列只算一次（月配另依間隔排下一期）；股數 0、沒有配息紀錄的持股不產生項目。
#[test]
fn duplicate_rows_and_empty_holdings() {
    let today = date(2026, 10, 1);
    let ex = Some(date(2026, 10, 16));
    let pay = Some(date(2026, 11, 13));
    let events = [
        event("00878", 2026, "Q3", dec!(0.4), ex, pay),
        event("00878", 2026, "M9", dec!(0.4), ex, pay),
    ];
    let holdings = [
        holding("00878", 1000, Some(dec!(20))),
        holding("9999", 500, Some(dec!(10))),
        holding("1101", 0, Some(dec!(30))),
    ];
    let forecast = forecast(&holdings, &events, &[], today, 3);
    // 已公告的兩列只算一次；月配另依間隔（只有一期時當作 1 個月）排出 11-16 除息那期。
    assert_eq!(
        summary(&forecast),
        [
            (
                date(2026, 10, 16),
                date(2026, 11, 13),
                dec!(400),
                Basis::Announced
            ),
            (
                date(2026, 11, 16),
                date(2026, 12, 14),
                dec!(400),
                Basis::Cadence
            ),
        ]
    );
    assert_eq!(forecast.announced_total, 400.0);
    assert_eq!(forecast.months.len(), 4);

    let empty = super::forecast(&[], &[], &[], today, 1);
    assert_eq!(
        (empty.total, empty.items.len(), empty.months.len()),
        (0.0, 0, 2)
    );
    assert_eq!(empty.forward_yield_percent, None);
}

fn summary(forecast: &Forecast) -> Vec<(NaiveDate, NaiveDate, Decimal, Basis)> {
    forecast
        .items
        .iter()
        .map(|item| (item.ex_date, item.pay_date, item.amount, item.basis))
        .collect()
}

/// 年配與半年配維持去年同期的金額，不套用最近一期。
#[test]
fn annual_and_half_yearly_keep_last_year_amount() {
    let today = date(2026, 3, 1);
    let events = [
        event(
            "2330",
            2025,
            "H1",
            dec!(4.0),
            Some(date(2025, 6, 12)),
            Some(date(2025, 7, 10)),
        ),
        event(
            "2330",
            2025,
            "H2",
            dec!(5.0),
            Some(date(2025, 12, 11)),
            Some(date(2026, 1, 8)),
        ),
        event(
            "1101",
            2024,
            "",
            dec!(1.0),
            Some(date(2025, 7, 1)),
            Some(date(2025, 8, 1)),
        ),
    ];
    let holdings = [holding("2330", 1000, None), holding("1101", 1000, None)];
    let forecast = forecast(&holdings, &events, &[], today, 12);

    let amounts: Vec<(&str, Decimal, Basis)> = forecast
        .items
        .iter()
        .map(|item| (item.stock_symbol.as_str(), item.amount, item.basis))
        .collect();
    assert_eq!(
        amounts,
        [
            ("2330", dec!(4000), Basis::Estimated),
            ("1101", dec!(1000), Basis::Estimated),
            ("2330", dec!(5000), Basis::Estimated),
        ]
    );
}

/// 新上市的月配 ETF 沒有去年同期：從最近一期依一個月的間隔往後排，發放日沿用最近一期
/// 「除息到發放」的 25 天，金額用最近一期。
#[test]
fn new_monthly_etf_follows_payout_cadence() {
    let today = date(2026, 11, 3);
    let events = [
        event(
            "00999",
            2026,
            "M8",
            dec!(0.1),
            Some(date(2026, 8, 18)),
            Some(date(2026, 9, 10)),
        ),
        event(
            "00999",
            2026,
            "M9",
            dec!(0.1),
            Some(date(2026, 9, 17)),
            Some(date(2026, 10, 14)),
        ),
        event(
            "00999",
            2026,
            "M10",
            dec!(0.12),
            Some(date(2026, 10, 20)),
            Some(date(2026, 11, 14)),
        ),
    ];
    let forecast = forecast(
        &[holding("00999", 1000, Some(dec!(10)))],
        &events,
        &[],
        today,
        3,
    );

    assert_eq!(
        summary(&forecast),
        [
            (
                date(2026, 10, 20),
                date(2026, 11, 14),
                dec!(120),
                Basis::Announced
            ),
            (
                date(2026, 11, 20),
                date(2026, 12, 15),
                dec!(120),
                Basis::Cadence
            ),
            (
                date(2026, 12, 20),
                date(2027, 1, 14),
                dec!(120),
                Basis::Cadence
            ),
        ]
    );
    assert_eq!(
        (forecast.announced_total, forecast.estimated_total),
        (120.0, 240.0)
    );
    let december = forecast
        .months
        .iter()
        .find(|month| month.month == "2026-12")
        .expect("12 月");
    assert_eq!((december.announced, december.estimated), (0.0, 120.0));
}

/// 月配有去年同期時：已推估的月份不再依間隔重複排；被已公告配息擋掉的相鄰月份由間隔補上。
#[test]
fn monthly_cadence_skips_months_already_forecast() {
    let today = date(2026, 3, 1);
    let events = [
        event(
            "00878",
            2025,
            "M3",
            dec!(0.1),
            Some(date(2025, 3, 18)),
            Some(date(2025, 4, 10)),
        ),
        event(
            "00878",
            2025,
            "M4",
            dec!(0.1),
            Some(date(2025, 4, 17)),
            Some(date(2025, 5, 12)),
        ),
        event(
            "00878",
            2025,
            "M5",
            dec!(0.1),
            Some(date(2025, 5, 19)),
            Some(date(2025, 6, 10)),
        ),
        event(
            "00878",
            2026,
            "M1",
            dec!(0.08),
            Some(date(2026, 1, 19)),
            Some(date(2026, 2, 10)),
        ),
        event(
            "00878",
            2026,
            "M2",
            dec!(0.09),
            Some(date(2026, 2, 17)),
            Some(date(2026, 3, 12)),
        ),
    ];
    let forecast = forecast(&[holding("00878", 1000, None)], &events, &[], today, 4);

    assert_eq!(
        summary(&forecast),
        [
            (
                date(2026, 2, 17),
                date(2026, 3, 12),
                dec!(90),
                Basis::Announced
            ),
            (
                date(2026, 3, 17),
                date(2026, 4, 9),
                dec!(90),
                Basis::Cadence
            ),
            (
                date(2026, 4, 17),
                date(2026, 5, 10),
                dec!(90),
                Basis::Cadence
            ),
            (
                date(2026, 5, 19),
                date(2026, 6, 10),
                dec!(90),
                Basis::LatestAmount
            ),
        ]
    );
}

/// 月配最近一期已超過兩個間隔沒有配息時視為停配，不再依間隔排。
#[test]
fn stale_monthly_payer_is_not_projected() {
    let today = date(2026, 3, 1);
    let events = [
        event(
            "00998",
            2025,
            "M10",
            dec!(0.1),
            Some(date(2025, 11, 3)),
            Some(date(2025, 11, 28)),
        ),
        event(
            "00998",
            2025,
            "M11",
            dec!(0.1),
            Some(date(2025, 12, 1)),
            Some(date(2025, 12, 26)),
        ),
    ];
    let forecast = forecast(&[holding("00998", 1000, None)], &events, &[], today, 3);

    assert!(forecast.items.is_empty(), "{:?}", forecast.items);
}

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
