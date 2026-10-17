//! 具名策略：回測各端點的 `preset=名稱` 參數，以及 `/strategy-presets` 清單端點。
//!
//! 帶 `preset` 時先讀出存的 query string，再用請求另帶的參數覆蓋同名參數（例如
//! `preset=最佳組合&rebalance_offset=3`），合併後與一般請求一樣驗證。策略由手動回補管理介面的
//! `/api/strategy-presets` 新增、修改與刪除（見 `interfaces::web::backfill_admin`）。

use axum::{
    Json,
    extract::{FromRequestParts, Query},
    http::{StatusCode, Uri, request::Parts},
    response::{IntoResponse, Response},
};

use super::Backtest;
use super::gateway::rejection;
use crate::domain::strategy::{StrategyPreset, StrategyPresetRepository};
use crate::infra::database::repository::strategy_preset::PgStrategyPresetRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, ScreenBacktestParams, StrategyPresetItem, StrategyPresetsResponse,
};
use crate::interfaces::web::data_api::handlers::error_response;

/// 引用具名策略的參數名稱。
pub(crate) const PRESET_KEY: &str = "preset";

/// 回測參數的 extractor：與 `Query<ScreenBacktestParams>` 相同，另支援 `preset=名稱`。
pub(crate) struct BacktestQuery(pub(crate) ScreenBacktestParams);

impl<S: Send + Sync> FromRequestParts<S> for BacktestQuery {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let raw = parts.uri.query().unwrap_or_default();
        let name = preset_name(raw);
        let query = match &name {
            Some(name) => {
                let preset = match PgStrategyPresetRepository::new().find(name).await {
                    Ok(Some(preset)) => preset,
                    Ok(None) => {
                        return Err(error_response(
                            StatusCode::NOT_FOUND,
                            &format!("查無策略「{name}」"),
                        ));
                    }
                    Err(error) => {
                        tracing::error!(?error, "讀取具名策略失敗");
                        return Err(error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "伺服器內部發生未預期錯誤",
                        ));
                    }
                };
                merge_query(&preset.query, raw)
            }
            None => raw.to_owned(),
        };
        let mut params = parse_params(&query)
            .map_err(|message| error_response(StatusCode::BAD_REQUEST, &message))?;
        params.preset = name;
        Ok(BacktestQuery(params))
    }
}

/// 把 query string 解析成回測參數；格式錯誤回錯誤訊息。
pub(crate) fn parse_params(query: &str) -> Result<ScreenBacktestParams, String> {
    let uri: Uri = format!("/?{query}")
        .parse()
        .map_err(|_| "query string 格式錯誤".to_owned())?;
    Query::<ScreenBacktestParams>::try_from_uri(&uri)
        .map(|Query(params)| params)
        .map_err(|error| error.body_text())
}

/// 拆成 `(鍵, 原始值)`，略過空項目；鍵與值保留 URL 編碼。
fn pairs(query: &str) -> impl Iterator<Item = (&str, &str)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| pair.split_once('=').unwrap_or((pair, "")))
}

/// 解碼 query string 的值（`+` 為空白）。
fn decode(value: &str) -> String {
    let value = value.replace('+', " ");
    urlencoding::decode(&value)
        .map(|decoded| decoded.into_owned())
        .unwrap_or(value)
}

/// 請求中 `preset` 的值（解碼後去掉前後空白）；沒有或空白回 `None`。
pub(crate) fn preset_name(query: &str) -> Option<String> {
    pairs(query)
        .find(|(key, _)| *key == PRESET_KEY)
        .map(|(_, value)| decode(value).trim().to_owned())
        .filter(|name| !name.is_empty())
}

/// 存的 query string 加上請求的參數：同名參數以請求為準（去掉存的那一個、請求的參數接在後面）；
/// `preset` 本身不保留。
pub(crate) fn merge_query(stored: &str, request: &str) -> String {
    let overrides: Vec<(&str, &str)> = pairs(request)
        .filter(|(key, _)| *key != PRESET_KEY)
        .collect();
    let mut merged: Vec<(&str, &str)> = pairs(stored)
        .filter(|(key, _)| *key != PRESET_KEY)
        .filter(|(key, _)| !overrides.iter().any(|(other, _)| other == key))
        .collect();
    merged.extend(overrides);
    merged
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// 檢查要存成具名策略的條件：不可再引用 `preset`、可以解析並通過與 `/stocks/screen/backtest` 相同的
/// 驗證；`alert` 為 true 時必須指定 `from`（換股日從起日起算，省略時每天都會變）。
///
/// # Errors
///
/// 不通過時回傳說明訊息。
pub(crate) async fn validate_strategy_query(query: &str, alert: bool) -> Result<(), String> {
    if pairs(query).any(|(key, _)| key == PRESET_KEY) {
        return Err("策略條件不可再引用 preset".to_owned());
    }
    let params = parse_params(query)?;
    if alert && params.from.is_none() {
        return Err("發換股通知的策略必須指定 from".to_owned());
    }
    if let Err(response) = Backtest::prepare(&params).await {
        return Err(rejection(*response).await);
    }
    if params
        .rebalance_offset
        .is_some_and(|offset| offset >= params.hold_days.unwrap_or(20))
    {
        return Err("rebalance_offset 必須介於 0 至 hold_days − 1".to_owned());
    }
    Ok(())
}

/// 全部具名策略。
///
/// # Errors
///
/// 驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/strategy-presets", tag = "data-api", responses((status = 200, body = StrategyPresetsResponse), (status = 401, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn strategy_presets() -> Response {
    match PgStrategyPresetRepository::new().list().await {
        Ok(presets) => Json(StrategyPresetsResponse {
            presets: presets.into_iter().map(item).collect(),
        })
        .into_response(),
        Err(error) => {
            tracing::error!(?error, "讀取策略清單失敗");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "伺服器內部發生未預期錯誤",
            )
        }
    }
}

/// 策略的輸出。
fn item(preset: StrategyPreset) -> StrategyPresetItem {
    StrategyPresetItem {
        name: preset.name,
        query: preset.query,
        description: preset.description,
        alert: preset.alert,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `preset` 的值會解碼（含 `+` 空白與中文）並去掉前後空白；沒有或空白為 None。
    #[test]
    fn preset_name_decodes_value() {
        assert_eq!(
            preset_name("limit=5&preset=%E6%9C%80%E4%BD%B3+A"),
            Some("最佳 A".to_owned())
        );
        assert_eq!(preset_name("preset=%20"), None);
        assert_eq!(preset_name("limit=5&&presets=x"), None);
        assert_eq!(preset_name(""), None);
    }

    /// 請求的參數覆蓋同名參數、其餘接在後面；`preset` 不保留（含存的 query 裡誤存的）。
    #[test]
    fn merge_query_lets_request_override() {
        assert_eq!(
            merge_query(
                "limit=20&hold_days=20&preset=x&from=2019-01-02",
                "preset=a&hold_days=60&rebalance_offset=3"
            ),
            "limit=20&from=2019-01-02&hold_days=60&rebalance_offset=3"
        );
        assert_eq!(merge_query("limit=20", "preset=a"), "limit=20");
        assert_eq!(merge_query("", "preset=a&flag"), "flag=");
    }

    /// 解析失敗回錯誤訊息；成功時與一般 query 相同。
    #[test]
    fn parse_params_reports_errors() {
        let params = parse_params("limit=5&from=2019-01-02").expect("可解析");
        assert_eq!(params.limit, Some(5));
        assert_eq!(params.from.as_deref(), Some("2019-01-02"));
        assert!(parse_params("limit=abc").is_err());
        assert!(parse_params("limit=5 6").is_err());
    }
}
