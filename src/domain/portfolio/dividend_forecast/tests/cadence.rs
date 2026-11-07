//! 依配息頻率推估：年配、半年配沿用去年同期，月配依發放節奏推估。

use super::*;

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
