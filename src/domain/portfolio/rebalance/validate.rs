//! 再平衡輸入檢查，從計算主體搬出以維持模組大小。

use super::{CurrentPosition, EtfLookThroughComponent, RebalanceParams};
use anyhow::{Result, ensure};
use std::collections::HashMap;

/// 檢查交易單位、容忍帶、現金、費率、持股與 ETF 成分權重，失敗時沿用既有錯誤。
pub(super) fn validate_inputs(
    positions: &[CurrentPosition],
    etf_components: &[EtfLookThroughComponent],
    params: &RebalanceParams,
) -> Result<()> {
    ensure!(
        matches!(params.lot_size, 1 | 1000),
        "交易單位須為 1 或 1000"
    );
    ensure!(
        params.tolerance_percent.is_finite() && (0.0..=100.0).contains(&params.tolerance_percent),
        "容忍帶須為 0 至 100 的有限數值"
    );
    ensure!(
        params.available_cash.is_finite() && params.available_cash >= 0.0,
        "現金須為非負有限數值"
    );
    ensure!(
        [params.fee_rate, params.tax_rate, params.fee_discount]
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
        "費率須為 0 至 1 的有限數值"
    );
    for position in positions {
        ensure!(
            position.shares >= 0
                && position.current_price.is_finite()
                && position.current_price > 0.0,
            "{} 缺少有效收盤價或股數不合法",
            position.symbol
        );
    }
    let mut coverage = HashMap::<&str, f64>::new();
    for comp in etf_components {
        ensure!(
            comp.weight_percent.is_finite() && (0.0..=100.0).contains(&comp.weight_percent),
            "ETF 成分權重不合法"
        );
        *coverage.entry(&comp.etf_symbol).or_default() += comp.weight_percent;
    }
    ensure!(
        coverage.values().all(|v| *v <= 100.000001),
        "ETF 成分權重合計超過 100%"
    );
    Ok(())
}
