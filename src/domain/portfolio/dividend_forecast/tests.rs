use rust_decimal_macros::dec;

use super::*;

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

/// 已公告的配息直接列入；去年同期推一年；已公告的同期不重複；全年彙總列不計。
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
        // 去年 Q4（2026-01-22）推到 2027-01-22、發放 2027-02-11。
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
    let forecast = forecast(&[holding("0056", 1000, Some(dec!(40)))], &events, today, 12);

    let summary: Vec<(NaiveDate, Decimal, Basis)> = forecast
        .items
        .iter()
        .map(|item| (item.pay_date, item.amount, item.basis))
        .collect();
    assert_eq!(
        summary,
        vec![
            (date(2026, 11, 11), dec!(1720), Basis::Announced),
            (date(2027, 2, 11), dec!(866), Basis::Estimated),
            // 去年 Q2（2026-07-21）推到 2027-07-21、發放 2027-08-10。
            (date(2027, 8, 10), dec!(1350), Basis::Estimated),
        ]
    );
    assert_eq!(forecast.total, 3936.0);
    assert_eq!(
        (forecast.announced_total, forecast.estimated_total),
        (1720.0, 2216.0)
    );
    // 市值 40,000 → 9.84%。
    assert_eq!(forecast.forward_yield_percent, Some(9.84));
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
    let forecast = forecast(&[holding("2887", 2000, None)], &events, today, 12);
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

/// 同一除息日的季別與月別兩列只算一次；股數 0、沒有配息紀錄的持股不產生項目。
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
    let forecast = forecast(&holdings, &events, today, 3);
    assert_eq!(forecast.items.len(), 1);
    assert_eq!(forecast.total, 400.0);
    assert_eq!(forecast.months.len(), 4);

    let empty = super::forecast(&[], &[], today, 1);
    assert_eq!(
        (empty.total, empty.items.len(), empty.months.len()),
        (0.0, 0, 2)
    );
    assert_eq!(empty.forward_yield_percent, None);
}
