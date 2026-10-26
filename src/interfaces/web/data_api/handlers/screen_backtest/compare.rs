//! `/stocks/screen/backtest/compare` handler：同一組條件改幾個參數並排比較。
//!
//! 共用參數與 `/stocks/screen/backtest` 相同（可用 `preset`）；每個 `scenario` 是一段 URL 編碼的
//! query 片段，覆蓋共用參數的同名參數（規則同具名策略的覆蓋，見 [`super::preset::merge_query`]）。
//! 各情境是一次完整的單次回測（含日頻路徑），經 [`super::limit::run_limited`] 受全程序併發上限約束。

use axum::{
    Json,
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};

use super::limit::run_limited;
use super::preset::{PRESET_KEY, merge_query, parse_params, resolve_query};
use super::summary::{summarize, yearly};
use super::{Backtest, Simulation};
use crate::interfaces::web::data_api::dto::{
    BacktestScenarioResult, ErrorBody, ScreenBacktestCompareParams, ScreenBacktestCompareResponse,
    ScreenBacktestParams,
};
use crate::interfaces::web::data_api::handlers::{database_error, error_response};

/// 情境參數名稱。
const SCENARIO_KEY: &str = "scenario";

/// 情境顯示名稱的參數名稱（只在情境片段內）。
const LABEL_KEY: &str = "label";

/// 情境數量下限與上限。
const MIN_SCENARIOS: usize = 2;
const MAX_SCENARIOS: usize = 4;

/// 單一情境片段的長度上限（解碼後的字元數）。
const MAX_SCENARIO_CHARS: usize = 500;

/// 比較用的免責聲明。
const DISCLAIMER: &str =
    "回測僅供研究參考，不構成任何投資建議，亦不會自動下單；過去表現不代表未來結果。";

/// 解析後的一組情境：顯示名稱、覆蓋參數與合併後的完整回測參數。
#[derive(Debug)]
pub(super) struct Scenario {
    pub(super) label: String,
    pub(super) overrides: String,
    pub(super) params: ScreenBacktestParams,
}

/// 比較請求的 extractor：讀出共用參數（含 `preset` 合併）與各情境。
pub(crate) struct CompareQuery(Vec<Scenario>);

impl<S: Send + Sync> FromRequestParts<S> for CompareQuery {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let raw = parts.uri.query().unwrap_or_default();
        let (fragments, base): (Vec<&str>, Vec<&str>) = raw
            .split('&')
            .filter(|pair| !pair.is_empty())
            .partition(|pair| pair.split_once('=').map_or(*pair, |(key, _)| key) == SCENARIO_KEY);
        let (base, preset) = resolve_query(&base.join("&"))
            .await
            .map_err(|response| *response)?;
        let fragments: Vec<String> = fragments
            .into_iter()
            .map(|pair| decode(pair.split_once('=').map_or("", |(_, value)| value)))
            .collect();
        parse_scenarios(&base, preset.as_deref(), &fragments)
            .map(CompareQuery)
            .map_err(|message| error_response(StatusCode::UNPROCESSABLE_ENTITY, &message))
    }
}

/// 解碼 query string 的值（`+` 為空白）。
fn decode(value: &str) -> String {
    let value = value.replace('+', " ");
    urlencoding::decode(&value)
        .map(|decoded| decoded.into_owned())
        .unwrap_or(value)
}

/// 依共用參數（已合併 `preset`）與各情境片段組出情境；數量、長度或內容不合法時回錯誤訊息。
pub(super) fn parse_scenarios(
    base: &str,
    preset: Option<&str>,
    fragments: &[String],
) -> Result<Vec<Scenario>, String> {
    if !(MIN_SCENARIOS..=MAX_SCENARIOS).contains(&fragments.len()) {
        return Err(format!(
            "scenario 必須有 {MIN_SCENARIOS} 至 {MAX_SCENARIOS} 組"
        ));
    }
    fragments
        .iter()
        .enumerate()
        .map(|(index, fragment)| {
            let fragment = fragment.trim();
            if fragment.is_empty() || fragment.chars().count() > MAX_SCENARIO_CHARS {
                return Err(format!(
                    "第 {} 組 scenario 不可空白或超過 {MAX_SCENARIO_CHARS} 字",
                    index + 1
                ));
            }
            let mut label = None;
            let mut overrides = Vec::new();
            for pair in fragment.split('&').filter(|pair| !pair.is_empty()) {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                match key {
                    LABEL_KEY => label = Some(decode(value).trim().to_owned()),
                    PRESET_KEY | SCENARIO_KEY => {
                        return Err(format!("第 {} 組 scenario 不可包含 {key}", index + 1));
                    }
                    _ => overrides.push(pair),
                }
            }
            let overrides = overrides.join("&");
            let mut params = parse_params(&merge_query(base, &overrides))
                .map_err(|message| format!("第 {} 組 scenario：{message}", index + 1))?;
            params.preset = preset.map(str::to_owned);
            let overrides = decode(&overrides);
            Ok(Scenario {
                label: label
                    .filter(|label| !label.is_empty())
                    .unwrap_or_else(|| overrides.clone()),
                overrides,
                params,
            })
        })
        .collect()
}

/// 並排比較 2～4 組回測情境。
///
/// 共用參數與 `/stocks/screen/backtest` 相同；`scenario` 重複 2～4 次，每次是一段 URL 編碼的 query
/// 片段（例如 `execution_price%3Dnext_open%26slippage_bps%3D20%26label%3D次日開盤`），覆蓋同名的共用
/// 參數。回傳各情境的整段彙總與依年度彙總。
///
/// # Errors
///
/// 情境數量不在 2～4、片段空白、過長、包含 `preset`／`scenario`，或合併後的參數不通過與
/// `/stocks/screen/backtest` 相同的驗證時回 422；查無具名策略或迄日前沒有相對強度資料回 404；
/// 驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/screen/backtest/compare", tag = "data-api", params(ScreenBacktestParams, ScreenBacktestCompareParams), responses((status = 200, body = ScreenBacktestCompareResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn screen_backtest_compare(CompareQuery(scenarios): CompareQuery) -> Response {
    let mut prepared = Vec::with_capacity(scenarios.len());
    for (index, scenario) in scenarios.iter().enumerate() {
        let backtest = match Backtest::prepare(&scenario.params).await {
            Ok(backtest) => backtest,
            Err(response) => return *response,
        };
        let offset = scenario.params.rebalance_offset.unwrap_or(0);
        if offset >= backtest.hold_days {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                &format!(
                    "第 {} 組 scenario：rebalance_offset 必須介於 0 至 hold_days − 1",
                    index + 1
                ),
            );
        }
        prepared.push((scenario, backtest, offset));
    }
    let results = run_limited(prepared.iter(), |(scenario, backtest, offset)| async move {
        backtest
            .simulate(*offset)
            .await
            .map(|Simulation { rows, costs, .. }| BacktestScenarioResult {
                label: scenario.label.clone(),
                overrides: scenario.overrides.clone(),
                execution_model: backtest.execution.as_str().to_owned(),
                slippage_bps: backtest.slippage_bps,
                fee_discount: backtest.fee_discount,
                etf_weight: backtest.etf_weight,
                rebalance_offset: *offset,
                summary: summarize(&rows, &costs, backtest.hold_days),
                years: yearly(&rows, &costs),
            })
    })
    .await;
    match results {
        Ok(scenarios) => Json(ScreenBacktestCompareResponse {
            scenarios,
            disclaimer: DISCLAIMER,
        })
        .into_response(),
        Err(error) => database_error(error),
    }
}

#[cfg(test)]
mod tests {
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
}
