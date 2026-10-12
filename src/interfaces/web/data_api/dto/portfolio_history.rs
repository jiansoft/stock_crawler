//! `/portfolio/history` 的 request、response 與 OpenAPI schema。
//!
//! 持股的時間加權報酬、月報酬、最大回撤與同期大盤比較；計算規則見
//! [`crate::domain::portfolio::performance`]。金額單位為新台幣元，百分比為 %。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::domain::portfolio::performance::{MonthReturn, Performance};

/// 持股績效 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct PortfolioHistoryParams {
    /// 會員編號；0（預設）為全部會員合計。
    #[param(minimum = 0, default = 0)]
    pub(crate) member_id: Option<i64>,
    /// 回看幾個月（以最新一筆市值往回推），預設 12，範圍 1–120。
    #[param(minimum = 1, maximum = 120, default = 12)]
    pub(crate) months: Option<u8>,
}

/// 單月績效。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct PortfolioMonth {
    /// 月份，格式 `YYYY-MM`。
    pub(crate) month: String,
    /// 該月最後一個交易日的市值。
    pub(crate) end_value: f64,
    /// 當月淨投入（買進成本 − 賣出所得）。
    pub(crate) net_contribution: f64,
    /// 當月時間加權報酬（%）。
    pub(crate) return_percent: f64,
    /// 同月加權指數報酬（%）；缺指數時為 `null`。
    pub(crate) benchmark_return_percent: Option<f64>,
}

impl From<MonthReturn> for PortfolioMonth {
    fn from(month: MonthReturn) -> Self {
        Self {
            month: month.month,
            end_value: month.end_value,
            net_contribution: month.net_contribution,
            return_percent: month.return_percent,
            benchmark_return_percent: month.benchmark_return_percent,
        }
    }
}

/// 持股績效結果；市值歷史少於兩個交易日時，除 `member_id`、`months` 外皆為 `null`／空陣列。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PortfolioHistoryResponse {
    /// 會員編號；0 為全部會員合計。
    pub(crate) member_id: i64,
    /// 期間最後一個交易日。
    pub(crate) data_as_of: Option<String>,
    /// 期間第一個交易日（基準日，不計報酬）。
    pub(crate) from: Option<String>,
    /// 基準日市值。
    pub(crate) start_value: Option<f64>,
    /// 最後一日市值。
    pub(crate) end_value: Option<f64>,
    /// 期間投入合計：買進成本；查全部會員時另含會員開始記錄當天的市值（視為投入）。
    pub(crate) buys: Option<f64>,
    /// 期間賣出所得合計。
    pub(crate) sells: Option<f64>,
    /// 時間加權報酬（%），只看股價、不含現金股利。
    pub(crate) return_percent: Option<f64>,
    /// 年化報酬（%）；期間未滿一年時為 `null`。
    pub(crate) annualized_return_percent: Option<f64>,
    /// 最大回撤（%，負數或 0）。
    pub(crate) max_drawdown_percent: Option<f64>,
    /// 最大回撤的高點日；沒有回撤時為 `null`。
    pub(crate) max_drawdown_peak: Option<String>,
    /// 最大回撤的低點日；沒有回撤時為 `null`。
    pub(crate) max_drawdown_trough: Option<String>,
    /// 同期加權指數報酬（%）；缺指數時為 `null`。
    pub(crate) benchmark_return_percent: Option<f64>,
    /// 超額報酬（報酬 − 大盤，百分點）；缺任一時為 `null`。
    pub(crate) excess_return_percent: Option<f64>,
    /// 各月績效，依月份由舊到新。
    pub(crate) months: Vec<PortfolioMonth>,
}

impl PortfolioHistoryResponse {
    /// 由領域績效組出回應；`performance` 為 `None` 時各欄位為 `null`。
    pub(crate) fn new(member_id: i64, performance: Option<Performance>) -> Self {
        let Some(p) = performance else {
            return Self {
                member_id,
                data_as_of: None,
                from: None,
                start_value: None,
                end_value: None,
                buys: None,
                sells: None,
                return_percent: None,
                annualized_return_percent: None,
                max_drawdown_percent: None,
                max_drawdown_peak: None,
                max_drawdown_trough: None,
                benchmark_return_percent: None,
                excess_return_percent: None,
                months: Vec::new(),
            };
        };
        Self {
            member_id,
            data_as_of: Some(p.to.to_string()),
            from: Some(p.from.to_string()),
            start_value: Some(p.start_value),
            end_value: Some(p.end_value),
            buys: Some(p.buys),
            sells: Some(p.sells),
            return_percent: Some(p.return_percent),
            annualized_return_percent: p.annualized_return_percent,
            max_drawdown_percent: Some(p.max_drawdown_percent),
            max_drawdown_peak: p.max_drawdown_peak.map(|date| date.to_string()),
            max_drawdown_trough: p.max_drawdown_trough.map(|date| date.to_string()),
            excess_return_percent: p
                .benchmark_return_percent
                .map(|benchmark| ((p.return_percent - benchmark) * 100.0).round() / 100.0),
            benchmark_return_percent: p.benchmark_return_percent,
            months: p.months.into_iter().map(Into::into).collect(),
        }
    }
}
