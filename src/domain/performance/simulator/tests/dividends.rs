//! 現金股利、股票股利與兩者交錯時的持股與報酬計算。

use super::*;

/// 1. 完全沒有股利時，三種口徑的結果必須完全一致。
#[test]
fn test_no_dividend_all_metrics_equal() {
    let case = Case {
        base_date: date(2020, 1, 2),
        end_date: date(2021, 1, 2),
        base_price: dec!(100),
        end_price: dec!(150),
        events: &[],
        corporate_actions: &[],
    };
    let result = run_with_price(&case, Some(dec!(120))).expect("應可計算");

    // 期初買入 100 股，期末 150 元 → 15,000 元。
    assert_eq!(result.price.end_shares, dec!(100));
    assert_eq!(result.price.end_value, dec!(15000));
    assert_eq!(result.price, result.total);
    assert_eq!(result.total, result.reinvested);
    assert_eq!(result.dividend_events, 0);
    assert_eq!(result.price.total_return_pct, dec!(50));
}

/// 2. 僅有現金股利：口徑 B 的現金累積正確，且口徑 A 的期末價值低於 B。
#[test]
fn test_cash_dividend_only() {
    let events = [event(Some(date(2020, 7, 1)), dec!(5), None, Decimal::ZERO)];
    let case = Case {
        base_date: date(2020, 1, 2),
        end_date: date(2021, 1, 2),
        base_price: dec!(100),
        end_price: dec!(100),
        events: &events,
        corporate_actions: &[],
    };
    // 再投入查價回傳 None，讓口徑 C 也走現金累積，方便與 B 對照。
    let result = run_with_price(&case, None).expect("應可計算");

    // 100 股 × 5 元 = 500 元現金。
    assert_eq!(result.total.cash_received, dec!(500));
    assert_eq!(result.total.end_shares, dec!(100));
    assert_eq!(result.total.end_value, dec!(10500));
    // 口徑 A 忽略股利，期末價值必然較低。
    assert_eq!(result.price.cash_received, Decimal::ZERO);
    assert_eq!(result.price.end_value, dec!(10000));
    assert!(result.price.end_value < result.total.end_value);
    assert_eq!(result.dividend_events, 1);
}

/// 3. 僅有股票股利：股數增加為「原股數 ×(1 + 股票股利 / 10)」。
#[test]
fn test_stock_dividend_only() {
    let events = [event(None, Decimal::ZERO, Some(date(2020, 7, 1)), dec!(1))];
    let case = Case {
        base_date: date(2020, 1, 2),
        end_date: date(2021, 1, 2),
        base_price: dec!(100),
        end_price: dec!(100),
        events: &events,
        corporate_actions: &[],
    };
    let result = run_with_price(&case, Some(dec!(90))).expect("應可計算");

    // 100 股配股 1 元（配股率 0.1）→ 110 股。
    assert_eq!(result.total.end_shares, dec!(110));
    assert_eq!(result.reinvested.end_shares, dec!(110));
    // 口徑 A 不理會配股，股數維持 100。
    assert_eq!(result.price.end_shares, dec!(100));
    assert!(result.price.end_shares < result.total.end_shares);
    assert_eq!(result.total.cash_received, Decimal::ZERO);
    assert_eq!(result.dividend_events, 1);
}

/// 4. 先配股、後配息：配息基數必須是「配股後」的股數。
///
/// 若錯誤地把所有配息一次加總（以期初股數為基數），會得到 200 元現金；
/// 正確依時序套用應為 110 股 × 2 元 = 220 元。
#[test]
fn test_stock_then_cash_uses_post_split_shares() {
    let events = [event(
        Some(date(2020, 8, 1)),
        dec!(2),
        Some(date(2020, 7, 1)),
        dec!(1),
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

    assert_eq!(result.total.end_shares, dec!(110));
    // 關鍵斷言：220 而非 200。
    assert_eq!(result.total.cash_received, dec!(220));
    assert_eq!(result.total.end_value, dec!(11220));
    // 同一筆事件雖產生兩個動作，僅計 1 次。
    assert_eq!(result.dividend_events, 1);
}

/// 5. 先配息、後配股：與第 4 題對照，配息基數為期初股數。
#[test]
fn test_cash_then_stock_uses_pre_split_shares() {
    let events = [event(
        Some(date(2020, 7, 1)),
        dec!(2),
        Some(date(2020, 8, 1)),
        dec!(1),
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

    assert_eq!(result.total.end_shares, dec!(110));
    // 配息在配股之前 → 100 股 × 2 元 = 200 元。
    assert_eq!(result.total.cash_received, dec!(200));
    assert_eq!(result.total.end_value, dec!(11200));
    assert_eq!(result.dividend_events, 1);
}

/// 6. 同一日同時除息與除權：約定先除息、後除權。
#[test]
fn test_same_day_cash_before_stock() {
    let same_day = date(2020, 7, 1);
    let events = [event(Some(same_day), dec!(2), Some(same_day), dec!(1))];
    let case = Case {
        base_date: date(2020, 1, 2),
        end_date: date(2021, 1, 2),
        base_price: dec!(100),
        end_price: dec!(100),
        events: &events,
        corporate_actions: &[],
    };
    let result = run_with_price(&case, None).expect("應可計算");

    // 先除息（100 股 × 2 = 200）再除權（→ 110 股）。
    assert_eq!(result.total.cash_received, dec!(200));
    assert_eq!(result.total.end_shares, dec!(110));
}

/// 6b. 多筆事件、現金與股票除權息日交錯，驗證混合排序而非分兩迴圈。
#[test]
fn test_interleaved_dates_across_events() {
    // 事件 A：除權 2020-03-01（配股 1 元）；除息 2020-09-01（現金 1 元）
    // 事件 B：除息 2020-06-01（現金 1 元）；除權 2020-12-01（配股 1 元）
    let events = [
        event(
            Some(date(2020, 9, 1)),
            dec!(1),
            Some(date(2020, 3, 1)),
            dec!(1),
        ),
        event(
            Some(date(2020, 6, 1)),
            dec!(1),
            Some(date(2020, 12, 1)),
            dec!(1),
        ),
    ];
    let case = Case {
        base_date: date(2020, 1, 2),
        end_date: date(2021, 1, 2),
        base_price: dec!(100),
        end_price: dec!(100),
        events: &events,
        corporate_actions: &[],
    };
    let result = run_with_price(&case, None).expect("應可計算");

    // 正確時序：03/01 配股 →110 股；06/01 配息 110 元；
    // 09/01 配息 110 元；12/01 配股 →121 股。
    assert_eq!(result.total.end_shares, dec!(121));
    assert_eq!(result.total.cash_received, dec!(220));
    assert_eq!(result.dividend_events, 2);
}

/// 7. 區間外的事件必須被排除：早於期初、等於期初、晚於期末皆不採計。
#[test]
fn test_events_outside_range_excluded() {
    let base = date(2020, 1, 2);
    let end = date(2021, 1, 2);
    let events = [
        // 早於期初。
        event(Some(date(2019, 12, 1)), dec!(5), None, Decimal::ZERO),
        // 等於期初（開區間，不採計）。
        event(Some(base), dec!(5), None, Decimal::ZERO),
        // 晚於期末。
        event(Some(date(2021, 2, 1)), dec!(5), None, Decimal::ZERO),
        // 等於期末（閉區間，採計）。
        event(Some(end), dec!(3), None, Decimal::ZERO),
    ];
    let case = Case {
        base_date: base,
        end_date: end,
        base_price: dec!(100),
        end_price: dec!(100),
        events: &events,
        corporate_actions: &[],
    };
    let result = run_with_price(&case, None).expect("應可計算");

    // 只有期末當日那筆生效：100 股 × 3 元 = 300 元。
    assert_eq!(result.total.cash_received, dec!(300));
    assert_eq!(result.dividend_events, 1);
}
