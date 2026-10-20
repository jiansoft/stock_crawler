use rust_decimal_macros::dec;

use super::*;

fn dividend(
    serial: i64,
    year: i32,
    year_of_dividend: i32,
    quarter: &str,
    cash: Decimal,
) -> PayoutDividend {
    PayoutDividend {
        serial,
        security_code: "4735".to_string(),
        year,
        year_of_dividend,
        quarter: quarter.to_string(),
        cash_dividend: cash,
        stock_dividend: Decimal::ZERO,
        sum: cash,
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        payout_eps: None,
        payout_period: None,
    }
}

fn earnings(year: i32, quarter: &str, eps: Decimal) -> PeriodEarnings {
    PeriodEarnings {
        security_code: "4735".to_string(),
        year,
        quarter: quarter.to_string(),
        earnings_per_share: eps,
    }
}

/// 豪展 2025～2026 年報與季報 EPS（正式庫與 Yahoo 核對過）。
fn hao_zhan_earnings() -> Vec<PeriodEarnings> {
    vec![
        earnings(2024, "Q1", dec!(-0.15)),
        earnings(2024, "Q2", dec!(0.62)),
        earnings(2024, "Q3", dec!(0.56)),
        earnings(2024, "Q4", dec!(0.58)),
        earnings(2024, "", dec!(1.62)),
        earnings(2025, "Q1", dec!(0.29)),
        earnings(2025, "Q2", dec!(0.51)),
        earnings(2025, "Q3", dec!(0.9)),
        earnings(2025, "Q4", dec!(0.74)),
        earnings(2025, "", dec!(2.46)),
        earnings(2026, "Q1", dec!(1.19)),
        earnings(2026, "Q2", dec!(3.18)),
    ]
}

fn find(result: &[PayoutRatios], serial: i64) -> &PayoutRatios {
    result
        .iter()
        .find(|ratios| ratios.serial == serial)
        .unwrap_or_else(|| panic!("serial {serial} 應該要有結果：{result:?}"))
}

/// 豪展 2026 年發放 2025Q4 0.5、2026Q1 1.0、2026Q2 1.5，合計 3.0。
/// 分母是這三季 EPS 0.74 + 1.19 + 3.18 = 5.11，不是 2025 年全年 EPS 2.46（那會算出 121.95%）。
#[test]
fn annual_total_of_quarterly_payer_uses_covered_quarters() {
    let dividends = vec![
        dividend(1, 2025, 2025, "Q3", dec!(0.7)),
        dividend(2, 2026, 2025, "Q4", dec!(0.5)),
        dividend(3, 2026, 2026, "Q1", dec!(1)),
        dividend(4, 2026, 2026, "Q2", dec!(1.5)),
        dividend(5, 2026, 2025, "", dec!(3)),
    ];

    let result = calculate_payout_ratios(&dividends, &hao_zhan_earnings());

    let total = find(&result, 5);
    assert_eq!(total.payout_eps, dec!(5.11));
    assert_eq!(total.payout_period, "2025Q4~2026Q2");
    assert_eq!(total.payout_ratio, dec!(58.7084));

    // 每一筆季配只涵蓋單季：Q4 前面已有 Q3 配息，不會把整年算進來。
    let q4 = find(&result, 2);
    assert_eq!(q4.payout_eps, dec!(0.74));
    assert_eq!(q4.payout_period, "2025Q4");
    assert_eq!(q4.payout_ratio, dec!(67.5676));
}

/// 1102 亞泥：來源把一年一次的股利標成 Q4，但同一所屬年度沒有更早的配息，
/// 這筆涵蓋的是全年盈餘，分母取年報 EPS，而不是 Q4 單季 EPS（那會算出 395%）。
#[test]
fn lone_q4_dividend_covers_whole_year() {
    let dividends = vec![
        dividend(1, 2026, 2025, "Q4", dec!(2.3)),
        dividend(2, 2026, 2025, "", dec!(2.3)),
    ];
    let earnings = vec![
        earnings(2025, "Q1", dec!(0.6)),
        earnings(2025, "Q2", dec!(0.8)),
        earnings(2025, "Q3", dec!(1)),
        earnings(2025, "Q4", dec!(0.58)),
        earnings(2025, "", dec!(3)),
    ];

    let result = calculate_payout_ratios(&dividends, &earnings);

    for serial in [1, 2] {
        let ratios = find(&result, serial);
        assert_eq!(ratios.payout_eps, dec!(3));
        assert_eq!(ratios.payout_period, "2025");
        assert_eq!(ratios.payout_ratio, dec!(76.6667));
    }
}

/// 同一所屬年度前面沒有配息時，Q3 那筆涵蓋 Q1～Q3（豪展 2024 年 Q1 虧損、Q1/Q2 都沒配）。
#[test]
fn first_dividend_of_year_covers_earlier_quarters() {
    let dividends = vec![dividend(1, 2024, 2024, "Q3", dec!(0.45))];

    let result = calculate_payout_ratios(&dividends, &hao_zhan_earnings());

    let q3 = find(&result, 1);
    assert_eq!(q3.payout_eps, dec!(1.03));
    assert_eq!(q3.payout_period, "2024Q1~Q3");
}

/// 一般年配息（只有一列、季別空白）：分母是股利所屬年度的年報 EPS。
#[test]
fn plain_annual_dividend_uses_annual_eps() {
    let dividends = vec![dividend(1, 2025, 2024, "", dec!(1.25))];

    let result = calculate_payout_ratios(&dividends, &hao_zhan_earnings());

    let annual = find(&result, 1);
    assert_eq!(annual.payout_eps, dec!(1.62));
    assert_eq!(annual.payout_period, "2024");
    assert_eq!(annual.payout_ratio, dec!(77.1605));
}

/// 半年配：H1 涵蓋 Q1～Q2，H2 涵蓋 Q3～Q4。
#[test]
fn half_year_dividends_cover_two_quarters() {
    let dividends = vec![
        dividend(1, 2025, 2025, "H1", dec!(0.4)),
        dividend(2, 2026, 2025, "H2", dec!(0.8)),
    ];

    let result = calculate_payout_ratios(&dividends, &hao_zhan_earnings());

    assert_eq!(find(&result, 1).payout_eps, dec!(0.8));
    assert_eq!(find(&result, 1).payout_period, "2025Q1~Q2");
    assert_eq!(find(&result, 2).payout_eps, dec!(1.64));
    assert_eq!(find(&result, 2).payout_period, "2025Q3~Q4");
}

/// 涵蓋期間缺任何一季的財報就不計算，保留原值等下次排程；年度合計也跟著不算。
#[test]
fn missing_quarter_leaves_row_untouched() {
    let dividends = vec![
        dividend(1, 2026, 2026, "Q3", dec!(1)),
        dividend(2, 2026, 2026, "Q2", dec!(1.5)),
        dividend(3, 2026, 2025, "", dec!(2.5)),
    ];

    let result = calculate_payout_ratios(&dividends, &hao_zhan_earnings());

    assert!(result.iter().all(|ratios| ratios.serial == 2), "{result:?}");
}

/// 虧損期間仍配息：分母照實寫回，分配率寫 0（負值會污染下游統計）。
#[test]
fn loss_period_writes_zero_ratio_with_actual_eps() {
    let dividends = vec![dividend(1, 2024, 2024, "Q1", dec!(0.3))];

    let result = calculate_payout_ratios(&dividends, &hao_zhan_earnings());

    let q1 = find(&result, 1);
    assert_eq!(q1.payout_eps, dec!(-0.15));
    assert_eq!(q1.payout_ratio, Decimal::ZERO);
}

/// 已經寫過相同結果的列不重複更新；沒有財報的股票（ETF）整檔略過。
#[test]
fn unchanged_rows_and_stocks_without_earnings_are_skipped() {
    let mut computed = dividend(1, 2025, 2024, "", dec!(1.25));
    computed.payout_ratio_cash = dec!(77.1605);
    computed.payout_ratio = dec!(77.1605);
    computed.payout_eps = Some(dec!(1.62));
    computed.payout_period = Some("2024".to_string());
    let mut etf = dividend(2, 2025, 2024, "", dec!(1));
    etf.security_code = "0056".to_string();

    let result = calculate_payout_ratios(&[computed, etf], &hao_zhan_earnings());

    assert!(result.is_empty(), "{result:?}");
}

/// 配股也各自算分配率：現金 7.5、配股 0.5，涵蓋期間 EPS 6.58（2753 的 25H2）。
#[test]
fn splits_cash_and_stock_ratios() {
    let mut row = dividend(1, 2026, 2025, "H2", dec!(7.5));
    row.security_code = "2753".to_string();
    row.stock_dividend = dec!(0.5);
    row.sum = dec!(8);
    let earnings = vec![
        PeriodEarnings {
            security_code: "2753".to_string(),
            year: 2025,
            quarter: "Q3".to_string(),
            earnings_per_share: dec!(3.2),
        },
        PeriodEarnings {
            security_code: "2753".to_string(),
            year: 2025,
            quarter: "Q4".to_string(),
            earnings_per_share: dec!(3.38),
        },
    ];
    // H2 前面沒有 H1 配息時會往前涵蓋整年；這裡補上 H1 讓 H2 只涵蓋下半年。
    let mut first_half = dividend(2, 2025, 2025, "H1", dec!(1));
    first_half.security_code = "2753".to_string();

    let result = calculate_payout_ratios(&[row, first_half], &earnings);

    let second_half = find(&result, 1);
    assert_eq!(second_half.payout_ratio_cash, dec!(113.9818));
    assert_eq!(second_half.payout_ratio_stock, dec!(7.5988));
    assert_eq!(second_half.payout_ratio, dec!(121.5805));
}

#[test]
fn period_label_formats() {
    let point = |year, quarter| QuarterPoint { year, quarter };
    let label = |start, end| {
        period_label(&Coverage {
            start,
            end,
            eps: Decimal::ONE,
        })
    };

    assert_eq!(label(point(2025, 1), point(2025, 4)), "2025");
    assert_eq!(label(point(2026, 2), point(2026, 2)), "2026Q2");
    assert_eq!(label(point(2026, 1), point(2026, 2)), "2026Q1~Q2");
    assert_eq!(label(point(2025, 4), point(2026, 2)), "2025Q4~2026Q2");
}
