//! 回測情境比較（`/stocks/screen/backtest/compare`）的參數與 response。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{BacktestSummary, BacktestYear};

/// 只用於 OpenAPI 文件：`scenario` 可重複 2～4 次，其餘參數與 `/stocks/screen/backtest` 相同。
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[allow(dead_code)]
pub(crate) struct ScreenBacktestCompareParams {
    /// 一組情境：URL 編碼過的 query 片段，覆蓋共用參數的同名參數，例如
    /// `execution_price%3Dnext_open%26slippage_bps%3D20`；可另帶 `label` 當顯示名稱。重複 2～4 次。
    #[param(example = "execution_price=next_open&slippage_bps=20&label=次日開盤＋20bps")]
    pub(crate) scenario: String,
}

/// 一組情境的回測結果。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestScenarioResult {
    /// 顯示名稱：情境的 `label`，沒有時為覆蓋參數本身。
    pub(crate) label: String,
    /// 這組情境覆蓋的參數（解碼後，不含 `label`）。
    pub(crate) overrides: String,
    /// 成交價模型（`close` 或 `next_open`）。
    pub(crate) execution_model: String,
    /// 每次買進、賣出各計的滑價（基點）。
    pub(crate) slippage_bps: u16,
    /// 手續費折扣。
    pub(crate) fee_discount: f64,
    /// 0050 核心部位（%）。
    pub(crate) etf_weight: u8,
    /// 換股日偏移。
    pub(crate) rebalance_offset: u16,
    /// 與 `/stocks/screen/backtest` 相同的整段彙總。
    pub(crate) summary: BacktestSummary,
    /// 依年度彙總。
    pub(crate) years: Vec<BacktestYear>,
}

/// 回測情境比較：同一組條件只改幾個參數（例如成交價模型、滑價、0050 比例），並排比較績效。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ScreenBacktestCompareResponse {
    /// 各情境依請求順序排列。
    pub(crate) scenarios: Vec<BacktestScenarioResult>,
    /// 試算免責聲明。
    pub(crate) disclaimer: &'static str,
}
