//! 分割、減資等公司行動調整持股數的測試。

use super::*;

/// 分割：三個口徑的持股都按比例換算，報酬率不受影響。
///
/// 這正是 0050 在 2025-06-18 的情形——期初 188.65、期末 47.57 看似
/// 大跌，實際上是 1 股換 4 股，持股價值幾乎不變。
#[test]
fn test_split_scales_shares_in_every_metric() {
    let actions = [corporate_action(date(2020, 6, 18), dec!(4))];
    let case = Case {
        corporate_actions: &actions,
        ..Case::plain(dec!(200), dec!(50))
    };

    let result = run_with_price(&case, None).expect("應可計算");

    // 期初 10000/200 = 50 股，分割後 200 股，期末 200 × 50 = 10000。
    assert_eq!(result.price.end_shares, dec!(200));
    assert_eq!(result.price.end_value, dec!(10000));
    assert_eq!(result.price.total_return_pct, Decimal::ZERO);
    assert_eq!(result.total.end_shares, dec!(200));
    assert_eq!(result.reinvested.end_shares, dec!(200));
    // 分割不是除權息，不列入事件數。
    assert_eq!(result.dividend_events, 0);
}

/// 沒有登錄分割時，同一情境會被算成 −75% —— 這正是要修正的錯誤。
#[test]
fn test_without_the_split_the_return_is_catastrophically_wrong() {
    let result = run_with_price(&Case::plain(dec!(200), dec!(50)), None).expect("應可計算");

    assert_eq!(result.price.end_shares, dec!(50));
    assert_eq!(result.price.end_value, dec!(2500));
    assert_eq!(result.price.total_return_pct, dec!(-75));
}

/// 減資：比例小於 1，股數等比例縮減。
#[test]
fn test_capital_reduction_shrinks_shares() {
    // 減資三成：1,000 股變 700 股，參考價相應上調。
    let actions = [corporate_action(date(2020, 6, 18), dec!(0.7))];
    let case = Case {
        corporate_actions: &actions,
        ..Case::plain(dec!(70), dec!(100))
    };

    let result = run_with_price(&case, None).expect("應可計算");

    // 期初 10000/70 股 × 0.7 = 100 股，期末 100 × 100 = 10000。
    assert_eq!(result.price.end_shares, dec!(100));
    assert_eq!(result.price.end_value, dec!(10000));
    assert_eq!(result.price.total_return_pct, Decimal::ZERO);
}

/// 區間外的公司行動不得生效，期初日當天生效者也不算。
#[test]
fn test_corporate_actions_outside_the_window_are_ignored() {
    let actions = [
        // 期初日之前。
        corporate_action(date(2019, 6, 1), dec!(4)),
        // 期初日當天：該日報價已是調整後價格，再乘一次會重複計算。
        corporate_action(date(2020, 1, 2), dec!(4)),
        // 期末日之後。
        corporate_action(date(2021, 6, 1), dec!(4)),
    ];
    let case = Case {
        corporate_actions: &actions,
        ..Case::plain(dec!(100), dec!(100))
    };

    let result = run_with_price(&case, None).expect("應可計算");

    assert_eq!(result.price.end_shares, dec!(100));
    assert_eq!(result.price.total_return_pct, Decimal::ZERO);
}

/// 期末日當天生效的分割仍要採計（左開右閉）。
#[test]
fn test_split_on_the_end_date_is_applied() {
    let actions = [corporate_action(date(2021, 1, 2), dec!(2))];
    let case = Case {
        corporate_actions: &actions,
        ..Case::plain(dec!(100), dec!(50))
    };

    let result = run_with_price(&case, None).expect("應可計算");

    assert_eq!(result.price.end_shares, dec!(200));
    assert_eq!(result.price.total_return_pct, Decimal::ZERO);
}

/// 同日除息與分割：現金股利以分割前的股數計算。
#[test]
fn test_dividend_on_the_split_date_uses_pre_split_shares() {
    let events = [DividendEvent {
        stock_symbol: "0050".to_string(),
        ex_dividend_date_cash: Some(date(2020, 6, 18)),
        ex_dividend_date_stock: None,
        cash_dividend: dec!(2),
        stock_dividend: Decimal::ZERO,
    }];
    let actions = [corporate_action(date(2020, 6, 18), dec!(4))];
    let case = Case {
        events: &events,
        corporate_actions: &actions,
        ..Case::plain(dec!(200), dec!(50))
    };

    let result = run_with_price(&case, None).expect("應可計算");

    // 除息基數是分割前的 50 股 → 現金 100 元；若誤用分割後的 200 股
    // 會得到 400 元，把配息灌成四倍。
    assert_eq!(result.total.cash_received, dec!(100));
    assert_eq!(result.total.end_shares, dec!(200));
    assert_eq!(result.dividend_events, 1);
}

/// 比例為零或負數的登錄錯誤一律略過，不得讓持股歸零。
#[test]
fn test_non_positive_ratio_is_ignored() {
    for ratio in [Decimal::ZERO, dec!(-2)] {
        let actions = [corporate_action(date(2020, 6, 18), ratio)];
        let case = Case {
            corporate_actions: &actions,
            ..Case::plain(dec!(100), dec!(100))
        };

        let result = run_with_price(&case, None).expect("應可計算");
        assert_eq!(result.price.end_shares, dec!(100), "ratio={ratio}");
    }
}
