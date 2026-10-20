//! 無效輸入、缺價、長期精度、年化報酬邊界與無效事件的測試。

use super::*;

/// 8. 非法輸入一律回傳 `None`。
#[test]
fn test_invalid_inputs_return_none() {
    let lookup = |_: NaiveDate| None;
    let base = SimulationInput {
        principal: dec!(10000),
        base_date: date(2020, 1, 2),
        end_date: date(2021, 1, 2),
        base_price: dec!(100),
        end_price: dec!(100),
        events: &[],
        corporate_actions: &[],
        reinvest_prices: &lookup,
    };

    // 期初價為零。
    let mut tmp = base.clone();
    tmp.base_price = Decimal::ZERO;
    assert!(simulate(&tmp).is_none());

    // 期末價為零。
    let mut tmp1 = base.clone();
    tmp1.end_price = Decimal::ZERO;
    assert!(simulate(&tmp1).is_none());

    // 期末日等於期初日。
    let mut tmp2 = base.clone();
    tmp2.end_date = tmp2.base_date;
    assert!(simulate(&tmp2).is_none());

    // 期末日早於期初日。
    let mut tmp3 = base.clone();
    tmp3.end_date = date(2019, 1, 2);
    assert!(simulate(&tmp3).is_none());

    // 投入金額非正數。
    let mut tmp4 = base.clone();
    tmp4.principal = Decimal::ZERO;
    assert!(simulate(&tmp4).is_none());

    // 負數價格。
    let mut tmp5 = base;
    tmp5.base_price = dec!(-1);
    assert!(simulate(&tmp5).is_none());
}

/// 9. 再投入口徑查無除息日價格時，該次股利退回現金累積（不可遺失）。
#[test]
fn test_reinvest_falls_back_to_cash_when_price_missing() {
    let with_price = date(2020, 4, 1);
    let without_price = date(2020, 10, 1);
    let events = [
        event(Some(with_price), dec!(2), None, Decimal::ZERO),
        event(Some(without_price), dec!(2), None, Decimal::ZERO),
    ];
    let lookup = move |d: NaiveDate| {
        if d == with_price {
            Some(dec!(50))
        } else {
            None
        }
    };
    let input = SimulationInput {
        principal: dec!(10000),
        base_date: date(2020, 1, 2),
        end_date: date(2021, 1, 2),
        base_price: dec!(100),
        end_price: dec!(100),
        events: &events,
        corporate_actions: &[],
        reinvest_prices: &lookup,
    };
    let result = simulate(&input).expect("應可計算");

    // 第一次：100 股 × 2 = 200 元，以 50 元買回 4 股 → 104 股。
    // 第二次：104 股 × 2 = 208 元，查無價格 → 累積現金。
    assert_eq!(result.reinvested.end_shares, dec!(104));
    assert_eq!(result.reinvested.cash_received, dec!(208));
    assert_eq!(
        result.reinvested.end_value,
        dec!(104) * dec!(100) + dec!(208)
    );
    // 口徑 B 全數累積現金：200 + 200 = 400。
    assert_eq!(result.total.cash_received, dec!(400));
    assert_eq!(result.dividend_events, 2);
}

/// 10. 長期間（10 年、20 次除息）驗證 Decimal 累加不失真與年化正確性。
#[test]
fn test_long_horizon_precision() {
    let base = date(2010, 1, 4);
    // 刻意讓日數差恰為 3650 天，使 years 正好等於 10。
    let end = base + Duration::days(3650);
    let events: Vec<DividendEvent> = (1..=20)
        .map(|i| {
            event(
                Some(base + Duration::days(180 * i)),
                dec!(1),
                None,
                Decimal::ZERO,
            )
        })
        .collect();
    let case = Case {
        base_date: base,
        end_date: end,
        base_price: dec!(100),
        end_price: dec!(200),
        events: &events,
        corporate_actions: &[],
    };
    let result = run_with_price(&case, None).expect("應可計算");

    assert_eq!(result.years, dec!(10));
    assert_eq!(result.dividend_events, 20);
    // 100 股不變（無配股），每次配息 100 元 × 20 次 = 2,000 元，且完全不失真。
    assert_eq!(result.total.end_shares, dec!(100));
    assert_eq!(result.total.cash_received, dec!(2000));
    assert_eq!(result.total.end_value, dec!(22000));
    assert_eq!(result.total.total_return_pct, dec!(120));

    // 手算年化：(22000 / 10000)^(1/10) - 1。
    let expected = ((2.2_f64).powf(0.1) - 1.0) * 100.0;
    let actual = result.total.cagr_pct.to_f64().expect("應可轉為 f64");
    assert!(
        (actual - expected).abs() < 1e-4,
        "年化報酬 {actual} 與手算 {expected} 差距過大"
    );
}

/// 11. `annualized_return_pct` 的邊界行為。
#[test]
fn test_annualized_return_pct_boundaries() {
    // years = 1 時，年化報酬等於區間總報酬。
    let cagr =
        annualized_return_pct(dec!(10000), dec!(12000), Decimal::ONE).expect("years = 1 應可計算");
    let total = total_return_pct(dec!(10000), dec!(12000)).expect("應可計算");
    assert!(
        (cagr - total).abs() < dec!(0.0001),
        "cagr={cagr} total={total}"
    );

    // 期末價值歸零 → -100%。
    assert_eq!(
        annualized_return_pct(dec!(10000), Decimal::ZERO, dec!(5)),
        Some(dec!(-100))
    );

    // 非法輸入。
    assert!(annualized_return_pct(Decimal::ZERO, dec!(100), dec!(1)).is_none());
    assert!(annualized_return_pct(dec!(-1), dec!(100), dec!(1)).is_none());
    assert!(annualized_return_pct(dec!(10000), dec!(100), Decimal::ZERO).is_none());
    assert!(annualized_return_pct(dec!(10000), dec!(100), dec!(-1)).is_none());
    assert!(annualized_return_pct(dec!(10000), dec!(-1), dec!(1)).is_none());

    // 總報酬率的非法輸入。
    assert!(total_return_pct(Decimal::ZERO, dec!(100)).is_none());
    assert!(total_return_pct(dec!(-5), dec!(100)).is_none());
    // 虧損情境。
    assert_eq!(total_return_pct(dec!(10000), dec!(2500)), Some(dec!(-75)));
}

/// 12. 兩個除權息日皆為 `None`（`sort_key()` 為 `None`）的事件被安全略過。
#[test]
fn test_event_without_any_date_is_skipped() {
    let events = [
        event(None, dec!(5), None, dec!(1)),
        event(Some(date(2020, 7, 1)), dec!(2), None, Decimal::ZERO),
    ];
    assert!(events[0].sort_key().is_none());

    let case = Case {
        base_date: date(2020, 1, 2),
        end_date: date(2021, 1, 2),
        base_price: dec!(100),
        end_price: dec!(100),
        events: &events,
        corporate_actions: &[],
    };
    let result = run_with_price(&case, None).expect("應可計算");

    // 只有第二筆生效。
    assert_eq!(result.total.cash_received, dec!(200));
    assert_eq!(result.total.end_shares, dec!(100));
    assert_eq!(result.dividend_events, 1);
}

/// 13. 金額為零的除權息（日期存在但金額為 0）不應被採計。
#[test]
fn test_zero_amount_event_not_counted() {
    let events = [event(
        Some(date(2020, 7, 1)),
        Decimal::ZERO,
        Some(date(2020, 7, 1)),
        Decimal::ZERO,
    )];
    let case = Case {
        base_date: date(2020, 1, 2),
        end_date: date(2021, 1, 2),
        base_price: dec!(100),
        end_price: dec!(100),
        events: &events,
        corporate_actions: &[],
    };
    let result = run_with_price(&case, None).expect("應可計算");

    assert_eq!(result.dividend_events, 0);
    assert_eq!(result.total.cash_received, Decimal::ZERO);
    assert_eq!(result.total.end_shares, dec!(100));
}
