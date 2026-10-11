//! `/stocks/{symbol}/financial-health` 的 request、response 與 OpenAPI schema。
//!
//! 以年度三大報表計算 Piotroski F 分數：9 項二元指標，通過一項得 1 分。
//! 每項都附上本期數值與比較基準，呼叫端可以自行檢查得分理由。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// F 分數的滿分。
pub(crate) const HEALTH_MAX_SCORE: u8 = 9;

/// 財務體質 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct FinancialHealthParams {
    /// 評分年度（西元），與前一年度比較；省略時取最新一個有年度損益表的年度。
    #[param(minimum = 2000, maximum = 2100)]
    pub(crate) fiscal_year: Option<i32>,
}

/// OpenAPI 文件使用的指標分類。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
pub(crate) enum HealthCategoryValue {
    /// 獲利能力（4 項）。
    Profitability,
    /// 財務槓桿與流動性（3 項）。
    LeverageLiquidity,
    /// 經營效率（2 項）。
    OperatingEfficiency,
}

/// OpenAPI 文件使用的數值單位。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
pub(crate) enum HealthValueUnitValue {
    /// 比率，0.25 即 25%。
    Ratio,
    /// 新台幣千元。
    ThousandTwd,
}

/// 單項 F 分數指標。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct HealthCriterion {
    /// 指標代碼，例如 `roa_positive`。
    pub(crate) key: &'static str,
    /// 指標名稱。
    pub(crate) name: &'static str,
    /// 指標分類。
    #[schema(value_type = HealthCategoryValue)]
    pub(crate) category: &'static str,
    /// 是否得分；資料不足或不適用（例如金融業沒有流動資產、流動負債的分類）為 `null`，不計分。
    pub(crate) passed: Option<bool>,
    /// 評分年度的數值。
    pub(crate) value: Option<f64>,
    /// 比較基準：前一年度的同一數值，或固定門檻 0，或同年度的稅後淨利（營業現金流一項）。
    pub(crate) baseline: Option<f64>,
    /// `value` 與 `baseline` 的單位。
    #[schema(value_type = HealthValueUnitValue)]
    pub(crate) unit: &'static str,
}

/// 財務體質評分結果。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct FinancialHealthResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 評分年度；沒有任何年度損益表時為 `null`。
    pub(crate) fiscal_year: Option<i32>,
    /// 比較的前一年度；`fiscal_year` 為 `null` 時也是 `null`。
    pub(crate) compared_with: Option<i32>,
    /// 得分：通過的指標數。
    pub(crate) score: u8,
    /// 可判定的指標數；資料不足的指標不計入，解讀分數時以此為分母。
    pub(crate) evaluated: u8,
    /// 滿分（9）。
    pub(crate) max_score: u8,
    /// 9 項指標，順序固定：獲利能力 4 項、財務槓桿與流動性 3 項、經營效率 2 項。
    pub(crate) criteria: Vec<HealthCriterion>,
}
