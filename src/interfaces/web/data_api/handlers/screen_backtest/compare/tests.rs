//! `/stocks/screen/backtest/compare` handler：同一組條件改幾個參數並排比較的測試。

use super::*;

fn fragments(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn scenarios_override_base_and_carry_labels() {
    let scenarios = parse_scenarios(
        "hold_days=20&limit=10&slippage_bps=5",
        None,
        &fragments(&[
            "execution_price=close",
            "execution_price=next_open&slippage_bps=20&label=%E6%AC%A1%E6%97%A5%E9%96%8B%E7%9B%A4",
        ]),
    )
    .expect("合法情境");
    assert_eq!(scenarios.len(), 2);
    assert_eq!(scenarios[0].label, "execution_price=close");
    assert_eq!(
        scenarios[0].params.slippage_bps,
        Some(5),
        "沒覆蓋的沿用共用參數"
    );
    assert_eq!(scenarios[1].label, "次日開盤");
    assert_eq!(
        scenarios[1].overrides,
        "execution_price=next_open&slippage_bps=20"
    );
    assert_eq!(scenarios[1].params.slippage_bps, Some(20));
    assert_eq!(
        scenarios[1].params.execution_price.as_deref(),
        Some("next_open")
    );
    assert_eq!(scenarios[1].params.limit, Some(10));
}

#[test]
fn scenarios_reject_bad_input() {
    for bad in [
        fragments(&["execution_price=close"]),
        fragments(&["a=1", "b=2", "c=3", "d=4", "e=5"]),
        fragments(&["execution_price=close", " "]),
        fragments(&["execution_price=close", "preset=x"]),
        fragments(&["execution_price=close", "scenario=x"]),
        fragments(&["execution_price=close", "limit=abc"]),
        vec!["execution_price=close".to_owned(), "x".repeat(501)],
    ] {
        assert!(parse_scenarios("", None, &bad).is_err(), "{bad:?}");
    }
}

#[test]
fn scenarios_keep_preset_name() {
    let scenarios = parse_scenarios(
        "hold_days=20",
        Some("最佳組合"),
        &fragments(&["slippage_bps=0", "slippage_bps=20"]),
    )
    .expect("合法情境");
    assert!(
        scenarios
            .iter()
            .all(|scenario| scenario.params.preset.as_deref() == Some("最佳組合"))
    );
}
