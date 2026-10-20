//! `/market/regime` handler：加權指數（TAIEX）的趨勢狀態。
//!
//! 一次讀資料日之前最近 [`regime::LOOKBACK`] 個交易日的收盤，以
//! [`crate::domain::market_index::regime`] 計算 5／20／60／120／240 日均線的位置與方向、均線排列、
//! 站在 60 日均線同一側的天數、52 週高低點距離、近 5／20／60／240 日漲跌幅與近
//! [`CROSS_WINDOW`] 個交易日穿越 [`CROSS_MA_DAYS`] 日均線的日子，用來判斷多空環境
//! （條件選股回測的 `market_filter_ma` 用的也是加權指數相對均線的位置）；另附同一天的市場廣度
//! （[`crate::domain::market_index::breadth`]），看大盤漲跌有沒有多數個股跟上；以及估值水位
//! （[`crate::domain::market_index::valuation::level`]），看市場是否偏貴。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::{error_response, parse_optional_date, repository_error, round2};
use crate::domain::market_index::breadth::{self, MarketBreadth};
use crate::domain::market_index::regime::{self, MarketRegime, MovingAverageCross};
use crate::domain::market_index::valuation::{self, ValuationLevel};
use crate::infra::database::repository::{
    market_index::PgMarketIndexRepository, market_valuation::PgMarketValuationRepository,
};
use crate::interfaces::web::data_api::dto::{
    ErrorBody, MarketRegimeBreadth, MarketRegimeParams, MarketRegimeResponse,
    MarketRegimeValuation, MovingAverageCrossItem, MovingAverageStatus,
};

/// 列出穿越的均線天數（5 日均線穿越太頻繁，不列）。
const CROSS_MA_DAYS: [u16; 4] = [20, 60, 120, 240];

/// 列出最近幾個交易日內的穿越。
const CROSS_WINDOW: usize = 60;

/// 市場廣度比較幾個交易日前、加總幾個交易日的新高新低。
const BREADTH_WINDOW: usize = 5;

/// 查詢加權指數的趨勢狀態。
///
/// # Errors
///
/// 日期格式錯誤回 422；資料日之前沒有加權指數回 404；驗證失敗回 401；資料庫查詢失敗回不含
/// SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/regime", tag = "data-api", params(MarketRegimeParams), responses((status = 200, body = MarketRegimeResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn market_regime(Query(params): Query<MarketRegimeParams>) -> Response {
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let repository = PgMarketIndexRepository::new();
    let points = match regime::load_taiex(&repository, date).await {
        Ok(points) => points,
        Err(error) => return repository_error(error),
    };
    let Some(state) = regime::regime(&points) else {
        return error_response(StatusCode::NOT_FOUND, "查無加權指數資料");
    };
    let breadth = match breadth::load(&repository, Some(state.date), BREADTH_WINDOW).await {
        Ok(breadth) => breadth,
        Err(error) => return repository_error(error),
    };
    // 估值水位只是附加資訊：讀取失敗只記錄，不讓整個趨勢狀態失敗。
    let level = valuation::level(
        &PgMarketValuationRepository::new(),
        state.date,
        BREADTH_WINDOW,
    )
    .await
    .unwrap_or_else(|why| {
        tracing::warn!("大盤趨勢狀態：讀取估值水位失敗: {why:#}");
        None
    });
    let mut body = response(&state, &recent_crosses(&points));
    body.breadth = breadth.as_ref().map(breadth_item);
    body.valuation = level.as_ref().map(valuation_item);
    Json(body).into_response()
}

/// 估值水位轉成回應；數值四捨五入到小數兩位。
fn valuation_item(level: &ValuationLevel) -> MarketRegimeValuation {
    MarketRegimeValuation {
        date: level.date.to_string(),
        median_pe: level.pe.map(|pe| round2(pe.current)),
        pe_percentile_5y: level.pe.map(|pe| round2(pe.percentile)),
        median_pe_5d_ago: level.pe_before.map(round2),
        median_pb: level.pb.map(|pb| round2(pb.current)),
        pb_percentile_5y: level.pb.map(|pb| round2(pb.percentile)),
    }
}

/// 市場廣度轉成回應；比例四捨五入到小數兩位。
fn breadth_item(breadth: &MarketBreadth) -> MarketRegimeBreadth {
    MarketRegimeBreadth {
        date: breadth.date.to_string(),
        above_ma20_percent: breadth.above_ma20_percent.map(round2),
        above_ma60_percent: breadth.above_ma60_percent.map(round2),
        above_ma240_percent: breadth.above_ma240_percent.map(round2),
        above_ma60_percent_5d_ago: breadth.above_ma60_percent_before.map(round2),
        new_highs_5d: breadth.new_highs,
        new_lows_5d: breadth.new_lows,
    }
}

/// 近 [`CROSS_WINDOW`] 個交易日穿越 [`CROSS_MA_DAYS`] 日均線的日子，由新到舊、同一天依均線天數。
fn recent_crosses(points: &[(chrono::NaiveDate, f64)]) -> Vec<MovingAverageCross> {
    let mut crosses: Vec<MovingAverageCross> = CROSS_MA_DAYS
        .iter()
        .flat_map(|&days| regime::crosses(points, days, CROSS_WINDOW))
        .collect();
    crosses.sort_by(|a, b| b.date.cmp(&a.date).then(a.days.cmp(&b.days)));
    crosses
}

/// 把趨勢狀態與穿越轉成回應；數值四捨五入到小數兩位；市場廣度另外填入。
fn response(state: &MarketRegime, crosses: &[MovingAverageCross]) -> MarketRegimeResponse {
    MarketRegimeResponse {
        data_as_of: state.date.to_string(),
        close: round2(state.close),
        change: state.change.map(round2),
        change_percent: state.change_percent.map(round2),
        alignment: state.alignment.as_str().to_owned(),
        moving_averages: state
            .moving_averages
            .iter()
            .map(|status| MovingAverageStatus {
                days: status.days,
                value: status.value.map(round2),
                distance_percent: status.distance_percent.map(round2),
                above: status.above,
                rising: status.rising,
            })
            .collect(),
        days_on_side_of_ma60: state.days_on_side_of_ma60,
        high_52_week: round2(state.high_52_week),
        low_52_week: round2(state.low_52_week),
        from_high_percent: round2(state.from_high_percent),
        from_low_percent: round2(state.from_low_percent),
        return_5d: state.return_5d.map(round2),
        return_20d: state.return_20d.map(round2),
        return_60d: state.return_60d.map(round2),
        return_240d: state.return_240d.map(round2),
        recent_crosses: crosses
            .iter()
            .map(|cross| MovingAverageCrossItem {
                date: cross.date.to_string(),
                days: cross.days,
                direction: if cross.above { "up" } else { "down" }.to_owned(),
            })
            .collect(),
        breadth: None,
        valuation: None,
    }
}

#[cfg(test)]
mod tests;
