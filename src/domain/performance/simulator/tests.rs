use super::*;
use chrono::Duration;
use rust_decimal_macros::dec;

/// 建立測試用日期。
fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
}

/// 建立除權息事件；日期為 `None` 表示該項不存在。
fn event(
    cash_date: Option<NaiveDate>,
    cash: Decimal,
    stock_date: Option<NaiveDate>,
    stock: Decimal,
) -> DividendEvent {
    DividendEvent {
        stock_symbol: "2330".to_string(),
        ex_dividend_date_cash: cash_date,
        ex_dividend_date_stock: stock_date,
        cash_dividend: cash,
        stock_dividend: stock,
    }
}

/// 建立模擬輸入的輔助結構（避免每個測試重複填欄位）。
struct Case<'a> {
    base_date: NaiveDate,
    end_date: NaiveDate,
    base_price: Decimal,
    end_price: Decimal,
    events: &'a [DividendEvent],
    /// 期間內的公司行動；多數案例為空。
    corporate_actions: &'a [CorporateAction],
}

impl Case<'_> {
    /// 一年期、無除權息也無公司行動的基準情境。
    fn plain(base_price: Decimal, end_price: Decimal) -> Self {
        Case {
            base_date: date(2020, 1, 2),
            end_date: date(2021, 1, 2),
            base_price,
            end_price,
            events: &[],
            corporate_actions: &[],
        }
    }
}

/// 建立一筆公司行動。
fn corporate_action(effective_date: NaiveDate, share_ratio: Decimal) -> CorporateAction {
    CorporateAction {
        action_type: crate::domain::performance::CorporateActionType::Split,
        stock_symbol: "0050".to_string(),
        effective_date,
        share_ratio,
        note: String::new(),
    }
}

/// 以「再投入永遠查得到固定價格」的方式執行模擬。
fn run_with_price(case: &Case<'_>, reinvest_price: Option<Decimal>) -> Option<SimulationResult> {
    let lookup = move |_: NaiveDate| reinvest_price;
    let input = SimulationInput {
        principal: dec!(10000),
        base_date: case.base_date,
        end_date: case.end_date,
        base_price: case.base_price,
        end_price: case.end_price,
        events: case.events,
        corporate_actions: case.corporate_actions,
        reinvest_prices: &lookup,
    };
    simulate(&input)
}

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
