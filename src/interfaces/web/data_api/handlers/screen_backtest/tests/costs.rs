//! 條件選股回測的交易成本、0050 混合與每期輸出測試。

use super::*;

/// 0050 核心部位：報酬與扣成本後報酬依比例混合、換手只算策略部位；空手與 0050 缺資料的期數照算；
/// 比例 0 不變、100 等於只持有 0050。
#[test]
fn blend_etf_mixes_core_and_strategy() {
    let mut rows = [
        dated(day(2024, 1, 2), Some(10.0), 0.0, Some(4.0)),
        dated(day(2024, 2, 1), None, 0.0, Some(-2.0)),
        dated(day(2024, 3, 1), Some(6.0), 0.0, None),
    ];
    let mut costs = [
        PeriodCost {
            turnover: 1.0,
            net_return: 9.8,
        },
        PeriodCost {
            turnover: 0.0,
            net_return: -0.4,
        },
        PeriodCost {
            turnover: 0.5,
            net_return: 5.9,
        },
    ];
    let (original_rows, original_costs) = (rows.clone(), costs);
    blend_etf(&mut rows, &mut costs, 0.0, 1.0);
    assert_eq!(
        (rows.clone(), costs),
        (original_rows.clone(), original_costs)
    );

    blend_etf(&mut rows, &mut costs, 0.25, 1.0);
    let avg: Vec<Option<f64>> = rows.iter().map(|row| row.avg_return).collect();
    assert_eq!(avg, [Some(8.5), Some(-0.5), Some(4.5)]);
    // 第一期 0050 第一次買進不計成本。
    assert!(
        near(costs[0].net_return, 0.25 * 4.0 + 0.75 * 9.8),
        "{costs:?}"
    );
    // 第一期策略漲 10%、0050 漲 4%：0050 比例掉到 0.26 ÷ 1.085，換股時減碼策略（手續費與證交稅）、
    // 買進 0050（手續費）。
    let drift = 0.25 - 0.26 / 1.085;
    assert!(
        near(
            costs[1].net_return,
            0.25 * -2.0 + 0.75 * -0.4 - drift * (0.1425 + 0.4425)
        ),
        "{costs:?}"
    );
    // 第二期策略空手：0050 跌 2% 後比例低於 0.25，減碼的是現金，只付買進 0050 的手續費。
    let cash_drift = 0.25 - 0.245 / 0.995;
    assert!(
        near(costs[2].net_return, 0.75 * 5.9 - cash_drift * 0.1425),
        "{costs:?}"
    );
    let turnover: Vec<f64> = costs.iter().map(|cost| cost.turnover).collect();
    assert!(near(turnover[0], 0.75), "{turnover:?}");
    assert!(near(turnover[1], drift), "{turnover:?}");
    assert!(near(turnover[2], 0.375 + cash_drift), "{turnover:?}");
    // 持股與報酬明細只看策略部位，不受影響。
    assert_eq!(rows[0].holdings, original_rows[0].holdings);

    // 比例 100% 不用調整，等於只持有 0050。
    let (mut rows, mut costs) = (original_rows.clone(), original_costs);
    blend_etf(&mut rows, &mut costs, 1.0, 1.0);
    let net: Vec<f64> = costs.iter().map(|cost| cost.net_return).collect();
    assert_eq!(net, [4.0, -2.0, 0.0]);

    // 0050 漲得比策略多：賣出 0050（手續費與 ETF 證交稅）、加碼策略（手續費）。
    let mut rows = [
        dated(day(2024, 1, 2), Some(0.0), 0.0, Some(10.0)),
        dated(day(2024, 2, 1), Some(1.0), 0.0, Some(0.0)),
    ];
    let mut costs = free(&rows);
    blend_etf(&mut rows, &mut costs, 0.5, 0.0);
    let excess = 0.55 / 1.05 - 0.5;
    assert!(
        near(
            costs[1].net_return,
            0.5 - excess * super::super::portfolio::ETF_TAX_RATE
        ),
        "{costs:?}"
    );
    assert!(near(costs[1].turnover, excess), "{costs:?}");
}

/// 換股成本：第一期全部買進；賣出付手續費與證交稅、買進付手續費；續抱且比例不變的不計，
/// 漲跌後調回目標比例的部分要計；空手時賣光。
#[test]
fn trading_costs_follow_turnover() {
    let rows = [
        holding(&["A", "B"], 10.0),
        holding(&["B", "C"], 5.0),
        holding(&["B", "C"], 1.0),
        row(3, 0, None, 0.0),
    ];
    let costs = trading_costs(&rows, 1.0);
    let turnover: Vec<f64> = costs.iter().map(|cost| cost.turnover).collect();
    assert_eq!(turnover, [1.0, 0.5, 0.0, 0.0]);
    // 第一期：買進全部，付 0.1425%。
    assert!(near(costs[0].net_return, 10.0 - 0.1425), "{costs:?}");
    // 第二期：賣出 A（一半）付 0.1425% + 0.3%，買進 C（一半）付 0.1425%。
    assert!(
        near(costs[1].net_return, 5.0 - 0.5 * 0.4425 - 0.5 * 0.1425),
        "{costs:?}"
    );
    assert!(near(costs[2].net_return, 1.0), "{costs:?}");
    // 空手：賣光上一期持股。
    assert!(near(costs[3].net_return, -0.4425), "{costs:?}");
    // 手續費 2.8 折只影響手續費。
    let discounted = trading_costs(&rows[..2], 0.28);
    assert!(
        near(discounted[0].net_return, 10.0 - 0.0399),
        "{discounted:?}"
    );
    assert!(
        near(
            discounted[1].net_return,
            5.0 - 0.5 * (0.0399 + 0.3) - 0.5 * 0.0399
        ),
        "{discounted:?}"
    );

    // 續抱股票漲跌後調回等權：A 漲 20%、B 持平，期末 A 占 0.6 ÷ 1.1，換股時減碼 A、加碼 B。
    let mut drifting = [holding(&["A", "B"], 10.0), holding(&["A", "B"], 2.0)];
    drifting[0].returns = vec![Some(20.0), Some(0.0)];
    let costs_drift = trading_costs(&drifting, 1.0);
    let trimmed = 0.6 / 1.1 - 0.5;
    assert!(near(costs_drift[1].turnover, trimmed), "{costs_drift:?}");
    assert!(
        near(
            costs_drift[1].net_return,
            2.0 - trimmed * 0.4425 - trimmed * 0.1425
        ),
        "{costs_drift:?}"
    );

    let summary = summarize(&rows, &costs, 20);
    // 換手率只看有持股的三期：(1 + 0.5 + 0) / 3。
    assert_eq!(summary.avg_turnover, Some(0.5));
    let expected = (1.0 + costs[0].net_return / 100.0)
        * (1.0 + costs[1].net_return / 100.0)
        * (1.0 + costs[2].net_return / 100.0)
        * (1.0 + costs[3].net_return / 100.0);
    assert_eq!(
        summary.total_return_after_costs,
        Some(round2((expected - 1.0) * 100.0))
    );
    assert!(summary.total_return_after_costs < summary.total_return);
}

/// 每期輸出：報酬四捨五入到小數兩位，空手為 `null`；代號只列前 10 檔。
#[test]
fn period_rounds_returns() {
    let mut first = row(2, 12, Some(1.234_5), -0.555);
    first.symbols = (0..12).map(|i| format!("{}", 2300 + i)).collect();
    let cost = PeriodCost {
        turnover: 1.0 / 3.0,
        net_return: 1.091_23,
    };
    let output = period((first, cost));
    assert_eq!(output.date, "2024-01-02");
    assert_eq!(output.avg_return, Some(1.23));
    assert_eq!(output.return_after_costs, 1.09);
    assert_eq!(output.turnover, 0.3333);
    assert_eq!(output.benchmark_return, Some(-0.56));
    assert_eq!(output.etf_return, None);
    assert_eq!(output.symbols.len(), 10);
    assert_eq!(output.symbols[0], "2300");
    let cash = PeriodCost {
        turnover: 0.0,
        net_return: 0.0,
    };
    assert_eq!(period((row(3, 0, None, 1.0), cash)).avg_return, None);
}

/// 期間停損：停損賣出的部位當期付手續費與證交稅、期末比例為 0，下次換股仍入選時視為重新買進；
/// 沒有停損時 [`portfolio::stopped_value`] 為 0。
#[test]
fn trading_costs_charge_stop_exits() {
    let mut first = holding(&["A", "B"], 0.0);
    first.returns = vec![Some(-10.0), Some(10.0)];
    first.stopped = vec![true, false];
    let mut second = holding(&["A", "B"], 0.0);
    second.returns = vec![Some(0.0), Some(0.0)];
    assert!(near(super::super::portfolio::stopped_value(&first), 0.45));
    assert!(near(super::super::portfolio::stopped_value(&second), 0.0));

    let costs = trading_costs(&[first, second], 1.0);
    assert!(
        near(costs[0].net_return, -0.1425 - 0.45 * 0.4425),
        "{costs:?}"
    );
    // A 已在期間賣出（期末比例 0）、B 期末 0.55 ÷ 1：買回 A 一半、B 減碼 0.05。
    assert!(near(costs[1].turnover, 0.5), "{costs:?}");
    assert!(
        near(costs[1].net_return, -0.05 * 0.4425 - 0.5 * 0.1425),
        "{costs:?}"
    );
}
