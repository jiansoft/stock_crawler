//! 策略優勢唯讀查詢；明確區分沒有快取、快取損壞與 Redis 故障。

use super::super::dto::{
    ErrorBody, StrategyEdgeItem, StrategyEdgeResponse, StrategyEdgeVerdict, StrategyEdgeWindow,
};
use super::error_response;
use crate::{
    app::event::taiwan_stock::strategy_edge::{self, EdgeRecord, EdgeWindow, Verdict},
    infra::nosql::redis::{self, RedisError},
};
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

/// 讀取每月第一個交易日更新的快取；不觸發回測、不寫入 Redis。
#[utoipa::path(get, path = "/api/v1/strategy-presets/edge", tag = "data-api", responses((status = 200, body = StrategyEdgeResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 500, body = ErrorBody), (status = 503, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn strategy_edge() -> Response {
    load_response(strategy_edge::REDIS_KEY).await
}

/// 以指定快取鍵組成 HTTP 回應；測試使用獨立鍵，避免動到最近一次正式檢查。
async fn load_response(key: &str) -> Response {
    cached_response(read_cache(key).await)
}

/// 以 Option 讀取 GET，讓不存在的鍵與 Redis 型別／連線錯誤明確分開。
async fn read_cache(key: &str) -> Result<String, RedisError> {
    let mut connection = redis::CLIENT.pool.get().await?;
    let value: Option<String> = deadpool_redis::redis::cmd("GET")
        .arg(key)
        .query_async(&mut connection)
        .await?;
    value.ok_or(RedisError::NotFound)
}

/// 將讀取結果轉成 HTTP 狀態，讓斷線與未命中可各自驗證。
fn cached_response(result: Result<String, RedisError>) -> Response {
    match result {
        Ok(json) => decode_response(&json),
        Err(RedisError::NotFound) => error_response(StatusCode::NOT_FOUND, "尚無策略優勢檢查紀錄"),
        Err(error) => {
            tracing::error!("策略優勢快取讀取失敗：{error:?}");
            error_response(StatusCode::SERVICE_UNAVAILABLE, "策略優勢快取暫時無法使用")
        }
    }
}

/// 解析快取並沿用排程的判斷規則，損壞時回不含原始內容的 500。
fn decode_response(json: &str) -> Response {
    match serde_json::from_str::<Vec<EdgeRecord>>(json) {
        Ok(records) if records.is_empty() => {
            error_response(StatusCode::NOT_FOUND, "尚無策略優勢檢查紀錄")
        }
        Ok(records) => Json(StrategyEdgeResponse {
            strategies: records
                .into_iter()
                .map(|record| {
                    let verdict = match strategy_edge::verdict(&record.recent) {
                        Verdict::Holds => StrategyEdgeVerdict::Holds,
                        Verdict::Weakened(_) => StrategyEdgeVerdict::Weakened,
                        Verdict::Unknown => StrategyEdgeVerdict::Unknown,
                    };
                    StrategyEdgeItem {
                        name: record.name,
                        checked: record.checked.to_string(),
                        recent: window(record.recent),
                        full: record.full.map(window),
                        verdict,
                    }
                })
                .collect(),
            execution_price: "next_open".to_owned(),
            slippage_bps: strategy_edge::SLIPPAGE_BPS,
            disclaimer: "僅供參考，不構成投資建議，亦不會自動下單".to_owned(),
        })
        .into_response(),
        Err(error) => {
            tracing::error!("策略優勢快取格式錯誤：{error}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "策略優勢快取格式錯誤")
        }
    }
}

/// 將應用層的期間分布轉成固定的 HTTP 契約。
fn window(value: EdgeWindow) -> StrategyEdgeWindow {
    StrategyEdgeWindow {
        from: value.from.to_string(),
        to: value.to.to_string(),
        offsets: value.offsets,
        median_return: value.median_return,
        etf_median_return: value.etf_median_return,
        beat_rate: value.beat_rate,
    }
}

#[cfg(test)]
mod tests;
