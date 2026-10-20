//! 財務體質評分的測試。
//!
//! 子模組：`scoring`（Piotroski 各項指標）、`snapshots`（快照與最新一季指標）；這裡放兩邊共用的
//! 兩年度健康數字。

mod scoring;
mod snapshots;

use rust_decimal_macros::dec;

use super::*;

/// 一般公司兩個年度的數值：除了長期負債比上升，其餘 8 項都改善。
pub(super) fn healthy_pair() -> (YearFigures, YearFigures) {
    let current = YearFigures {
        revenue: Some(4_600),
        gross_profit: Some(1_900),
        net_income: Some(460),
        operating_cash_flow: Some(900),
        total_assets: Some(52_000),
        current_assets: Some(20_000),
        current_liabilities: Some(10_000),
        non_current_liabilities: Some(11_000),
        share_capital: Some(1_000),
    };
    let previous = YearFigures {
        revenue: Some(4_000),
        gross_profit: Some(1_500),
        net_income: Some(300),
        operating_cash_flow: Some(200),
        total_assets: Some(48_000),
        current_assets: Some(15_000),
        current_liabilities: Some(9_000),
        non_current_liabilities: Some(10_000),
        share_capital: Some(1_000),
    };
    (current, previous)
}
