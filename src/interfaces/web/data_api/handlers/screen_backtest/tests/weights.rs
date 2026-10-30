//! 條件選股回測的持股加權方式與單檔權重上限測試。

use super::*;

/// 加權方式的參數值雙向對照；不認得的值為 `None`。
#[test]
fn weighting_parses_and_prints() {
    for (name, weighting) in Weighting::ALL {
        assert_eq!(Weighting::parse(name), Some(weighting));
        assert_eq!(weighting.as_str(), name);
    }
    assert_eq!(Weighting::parse("market_cap"), None);
    assert_eq!(Weighting::parse("EQUAL"), None);
}

/// 持股權重：等權各為 1；依成交金額或其平方根；缺值與非正值以最小值補；全部缺值改等權。
#[test]
fn holding_weights_follow_trade_value() {
    use super::super::portfolio::holding_weights;
    let values = [Some(400.0), None, Some(100.0), Some(0.0)];
    assert_eq!(
        holding_weights(&values, Weighting::Equal),
        [1.0, 1.0, 1.0, 1.0]
    );
    assert_eq!(
        holding_weights(&values, Weighting::TradeValue),
        [400.0, 100.0, 100.0, 100.0]
    );
    assert_eq!(
        holding_weights(&values, Weighting::SqrtTradeValue),
        [20.0, 10.0, 10.0, 10.0]
    );
    assert_eq!(
        holding_weights(&[None, None], Weighting::TradeValue),
        [1.0, 1.0]
    );
    assert!(holding_weights(&[], Weighting::TradeValue).is_empty());
}

/// 依成交金額加權：平均報酬為有報酬持股的加權平均（沒有報酬的不列入）；等權不變；個股貢獻與換手
/// 都依權重計算。
#[test]
fn weigh_uses_trade_value_for_returns_costs_and_contributors() {
    use super::super::contributors::contributors;
    use super::super::portfolio::weigh;
    let rows = select_holdings(
        vec![
            CandidateRow {
                candidate_trade_values: vec![Some(300.0), Some(100.0), Some(50.0)],
                ..candidates(2, &[("A", Some(10.0)), ("B", Some(-2.0)), ("C", None)])
            },
            CandidateRow {
                candidate_trade_values: vec![Some(300.0), Some(100.0)],
                ..candidates(3, &[("A", Some(4.0)), ("D", Some(8.0))])
            },
        ],
        3,
        3,
        None,
    );
    assert_eq!(rows[0].trade_values, [Some(300.0), Some(100.0), Some(50.0)]);
    let mut equal = rows.clone();
    weigh(&mut equal, Weighting::Equal, None);
    assert_eq!(equal, rows);
    assert_eq!(rows[0].avg_return, Some(4.0));

    let mut weighted = rows;
    weigh(&mut weighted, Weighting::TradeValue, None);
    assert_eq!(weighted[0].weights, [300.0, 100.0, 50.0]);
    // (10×300 − 2×100) ÷ 400 = 7；C 沒有報酬，不列入。
    assert_eq!(weighted[0].avg_return, Some(7.0));
    // (4×300 + 8×100) ÷ 400 = 5。
    assert_eq!(weighted[1].avg_return, Some(5.0));

    // 第一期期末：A 300×1.1、B 100×0.98、C（沒有報酬，視為平均 7%）50×1.07，合計 481.5。
    // 第二期 A 調到 0.75（加碼）、買進 D 0.25，賣出 B、C。
    let costs = trading_costs(&weighted, 0.0);
    assert!(near(costs[0].turnover, 1.0), "{costs:?}");
    let a_before = 330.0 / 481.5;
    assert!(near(costs[1].turnover, 0.75 - a_before + 0.25), "{costs:?}");
    assert!(
        near(
            costs[1].net_return,
            5.0 - (98.0 + 53.5) / 481.5 * super::super::portfolio::TAX_RATE
        ),
        "{costs:?}"
    );

    let (held, best, worst) = contributors(&weighted, 10);
    assert_eq!(held, 4);
    // A：10×300÷400 + 4×300÷400 = 10.5；D：8×100÷400 = 2；B：−2×100÷400 = −0.5。
    assert_eq!(
        best.iter()
            .map(|item| (item.symbol.as_str(), item.contribution))
            .collect::<Vec<_>>(),
        [("A", 10.5), ("D", 2.0)]
    );
    assert_eq!(
        worst
            .iter()
            .map(|item| (item.symbol.as_str(), item.contribution))
            .collect::<Vec<_>>(),
        [("B", -0.5)]
    );
}

/// 單檔上限：超過的固定在上限、多出的依比例分給其他持股，重複到沒有超過；檔數 × 上限不到 1 時放現金。
#[test]
fn cap_weights_redistributes_and_keeps_cash() {
    use super::super::portfolio::cap_weights;
    let close = |actual: &[f64], expected: &[f64]| {
        actual.len() == expected.len() && actual.iter().zip(expected).all(|(a, b)| near(*a, *b))
    };
    // 沒有超過上限：只正規化。
    let (weights, cash) = cap_weights(&[3.0, 1.0], 0.8);
    assert!(close(&weights, &[0.75, 0.25]), "{weights:?}");
    assert_eq!(cash, 0.0);
    // 第一檔 0.8 → 0.5，多出的 0.3 依 1:1 分給其他兩檔（各 0.25）；不再超過上限。
    let (weights, cash) = cap_weights(&[8.0, 1.0, 1.0], 0.5);
    assert!(close(&weights, &[0.5, 0.25, 0.25]), "{weights:?}");
    assert!(near(cash, 0.0), "{cash}");
    // 第一輪只有 A 超過；分配後 B 也超過，再固定 B，剩下的全給 C。
    let (weights, cash) = cap_weights(&[6.0, 3.0, 1.0], 0.4);
    assert!(close(&weights, &[0.4, 0.4, 0.2]), "{weights:?}");
    assert!(near(cash, 0.0), "{cash}");
    // 兩檔、上限 30%：各 30%，現金 40%。
    let (weights, cash) = cap_weights(&[1.0, 1.0], 0.3);
    assert!(close(&weights, &[0.3, 0.3]), "{weights:?}");
    assert!(near(cash, 0.4), "{cash}");
    // 沒有持股：沒有現金。
    assert_eq!(cap_weights(&[], 0.3), (Vec::new(), 0.0));
}

/// 等權搭配單檔上限：持股太少時放現金，平均報酬、換手、個股貢獻與目前持股權重都只算持股的部分。
#[test]
fn weigh_with_max_weight_holds_cash() {
    use super::super::contributors::contributors;
    use super::super::portfolio::{shares, weigh};
    let mut rows = vec![
        holding(&["A", "B"], 0.0),
        holding(&["A", "B", "C", "D"], 0.0),
    ];
    rows[0].returns = vec![Some(10.0), Some(-2.0)];
    rows[1].returns = vec![Some(4.0), Some(8.0), Some(0.0), None];
    weigh(&mut rows, Weighting::Equal, Some(0.25));
    // 兩檔各 25%、現金 50%：(10 − 2) ÷ 2 × 0.5 = 2。
    assert!(near(rows[0].cash, 0.5), "{:?}", rows[0]);
    assert_eq!(rows[0].avg_return, Some(2.0));
    assert!(
        shares(&rows[0]).iter().all(|share| near(*share, 0.25)),
        "{:?}",
        shares(&rows[0])
    );
    // 四檔各 25%，沒有現金；D 沒有報酬，平均 (4 + 8 + 0) ÷ 3 = 4。
    assert!(near(rows[1].cash, 0.0), "{:?}", rows[1]);
    assert!(
        rows[1].avg_return.is_some_and(|avg| near(avg, 4.0)),
        "{:?}",
        rows[1]
    );

    // 第一期買進一半（另一半是現金）；期末 A 0.25×1.1、B 0.25×0.98、現金 0.5，合計 1.02。
    // 第二期 A、B 調到 0.25，再買進 C、D 各 0.25。
    let costs = trading_costs(&rows, 0.0);
    assert!(near(costs[0].turnover, 0.5), "{costs:?}");
    let a_before = 0.275 / 1.02;
    let b_before = 0.245 / 1.02;
    assert!(near(costs[1].turnover, 0.25 - b_before + 0.5), "{costs:?}");
    assert!(
        near(
            costs[1].net_return,
            4.0 - (a_before - 0.25) * super::super::portfolio::TAX_RATE
        ),
        "{costs:?}"
    );

    // 第一期持股只占一半：A 10 × 1/2 × 0.5、B −2 × 1/2 × 0.5；第二期 A、B 各占有報酬持股的 1/3。
    let (_, best, worst) = contributors(&rows, 10);
    assert_eq!(
        best.iter()
            .map(|item| (item.symbol.as_str(), item.contribution))
            .collect::<Vec<_>>(),
        [
            ("A", round2(2.5 + 4.0 / 3.0)),
            ("B", round2(-0.5 + 8.0 / 3.0))
        ]
    );
    assert!(worst.is_empty(), "{worst:?}");

    // 依成交金額加權搭配上限：與沒有上限時相同的持股，只是權重被壓到上限。
    let mut weighted = vec![PeriodRow {
        trade_values: vec![Some(900.0), Some(100.0)],
        returns: vec![Some(10.0), Some(0.0)],
        ..holding(&["A", "B"], 0.0)
    }];
    weigh(&mut weighted, Weighting::TradeValue, Some(0.6));
    assert!(near(weighted[0].weights[0], 0.6), "{weighted:?}");
    assert!(near(weighted[0].cash, 0.0), "{weighted:?}");
    assert!(
        weighted[0].avg_return.is_some_and(|avg| near(avg, 6.0)),
        "{weighted:?}"
    );
}

/// 只符合續抱條件的持股沒有成交金額（SQL 不查），列為待補查；補查後填回，候選股原有的值不動，
/// 查不到的維持 `None`。
#[test]
fn hold_only_holdings_get_trade_values_after_selection() {
    use std::collections::HashMap;

    use super::super::portfolio::{fill_trade_values, missing_trade_values};
    let rows = select_holdings(
        vec![
            CandidateRow {
                candidate_trade_values: vec![Some(300.0), Some(100.0)],
                ..candidates(2, &[("A", Some(1.0)), ("B", Some(2.0))])
            },
            CandidateRow {
                candidate_trade_values: vec![Some(500.0)],
                ..with_hold(
                    candidates(3, &[("C", Some(3.0))]),
                    &[("A", Some(4.0)), ("B", Some(5.0))],
                )
            },
        ],
        3,
        3,
        None,
    );
    // 第二期：候選股 C 有成交金額；續抱的 A、B 只在續抱名單，沒有。
    assert_eq!(rows[1].symbols, ["C", "A", "B"]);
    assert_eq!(rows[1].trade_values, [Some(500.0), None, None]);
    assert_eq!(
        missing_trade_values(&rows),
        [
            (day(2024, 1, 3), "A".to_owned()),
            (day(2024, 1, 3), "B".to_owned())
        ]
    );

    let mut filled = rows.clone();
    fill_trade_values(
        &mut filled,
        &HashMap::from([
            ((day(2024, 1, 3), "A".to_owned()), 250.0),
            ((day(2024, 1, 3), "C".to_owned()), 1.0),
        ]),
    );
    assert_eq!(filled[1].trade_values, [Some(500.0), Some(250.0), None]);
    assert_eq!(filled[0].trade_values, rows[0].trade_values);

    let mut unknown = rows;
    fill_trade_values(&mut unknown, &HashMap::new());
    assert_eq!(unknown[1].trade_values, [Some(500.0), None, None]);
}
