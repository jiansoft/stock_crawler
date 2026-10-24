//! ETF 相對代理基準的追蹤誤差；明確區分追蹤指數名稱及實際比較基準。
use super::{error_response, etf::valid_symbol, repository_error};
use crate::interfaces::web::data_api::dto::ErrorBody;
use axum::{
    Json,
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub(crate) struct EtfTrackingResponse {
    stock_symbol: String,
    benchmark_symbol: String,
    benchmark_kind: String,
    target_benchmark: String,
    data_as_of: Option<String>,
    samples: i32,
    tracking_error_percent: Option<f64>,
    tracking_difference_percent: Option<f64>,
    note: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejects_invalid_symbol_before_database_access() {
        for symbol in ["", "0050;DROP", "xxxxxxxxxxxxx", "台積電"] {
            assert_eq!(
                etf_tracking(Path(symbol.into())).await.status(),
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }
    }
}

/// 讀取每日績效排程的近一年結果；資料不足時回 null，不以折溢價當作追蹤誤差。
#[utoipa::path(get,path="/api/v1/etfs/{symbol}/tracking-error",tag="data-api",params(("symbol"=String,Path)),responses((status=200,body=EtfTrackingResponse),(status=401,body=ErrorBody),(status=404,body=ErrorBody),(status=422,body=ErrorBody),(status=500,body=ErrorBody)),security(("bearer_auth"=[])))]
pub(crate) async fn etf_tracking(Path(symbol): Path<String>) -> Response {
    if !valid_symbol(&symbol) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "ETF 代號格式不正確");
    }
    let symbol = symbol.to_ascii_uppercase();
    let row = crate::infra::database::repository::etf::tracking::find(&symbol).await;
    match row{
        Ok(Some(row))=>Json(EtfTrackingResponse{stock_symbol:symbol,benchmark_symbol:"0050".into(),benchmark_kind:"etf_proxy".into(),target_benchmark:row.target_benchmark,data_as_of:row.calc_date.map(|d|d.to_string()),samples:row.samples.unwrap_or(0),tracking_error_percent:row.tracking_error_percent,tracking_difference_percent:row.tracking_difference_percent,note:"代理基準報酬差異：與 0050 含息日報酬比較；不是相對實際標的指數的追蹤誤差。至少 120 個相同區間日報酬才計算；累積報酬差僅涵蓋配對成功的區間。".into()}).into_response(),
        Ok(None)=>error_response(StatusCode::NOT_FOUND,"查無這檔 ETF"),
        Err(error)=>repository_error(error),
    }
}
