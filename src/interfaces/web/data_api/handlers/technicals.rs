//! `/stocks/{symbol}/technicals` handler：均線、乖離率、RSI、KD、MACD 與技術訊號。
//!
//! 日線來自 `DailyQuotes`（只讀覆蓋索引），依股利與公司行動事件還原權值後計算，規則在
//! [`crate::domain::quote::technical`]。為讓年線與 EMA／KD 的起始值穩定，
//! 會比請求天數多讀 [`technical::WARMUP_BARS`] 個交易日。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::{ensure_stock_exists, error_response, parse_optional_date, repository_error, round2};
use crate::domain::quote::technical::{
    self, IndicatorPoint, PriceAdjustment, PriceBar, PriceBarRepository,
};
use crate::infra::database::repository::quote::PgQuoteRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, TechnicalIndicatorsParams, TechnicalIndicatorsResponse, TechnicalPoint,
    TechnicalSignalItem,
};

/// 查詢單一股票截止日之前最近幾個交易日的技術指標與最新一日的訊號。
///
/// 價格已還原權值（最新一日為實際價格，除權息、減資、分割之前的價格依參考價調整）；
/// 樣本不足的指標為 `null`（例如上市未滿 240 個交易日沒有年線）。
///
/// # Errors
///
/// `days` 不在 1–120 或日期格式錯誤回 422、股票不存在回 404、驗證失敗回 401；
/// 資料庫查詢失敗時記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/technicals", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), TechnicalIndicatorsParams), responses((status = 200, body = TechnicalIndicatorsResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn stock_technicals(
    Path(symbol): Path<String>,
    Query(params): Query<TechnicalIndicatorsParams>,
) -> Response {
    let days = params.days.unwrap_or(20);
    if !(1..=120).contains(&days) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 1 至 120");
    }
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }

    let repo = PgQuoteRepository::new();
    let limit = i64::from(days) + technical::WARMUP_BARS as i64;
    let bars = match repo.fetch_price_bars(&symbol, date, limit).await {
        Ok(bars) => bars,
        Err(error) => return repository_error(error),
    };
    let adjustments = match repo
        .fetch_price_adjustments(std::slice::from_ref(&symbol))
        .await
    {
        Ok(mut map) => map.remove(&symbol).unwrap_or_default(),
        Err(error) => return repository_error(error),
    };

    Json(build_response(symbol, days, &bars, &adjustments)).into_response()
}

/// 由舊到新的日線還原權值後計算指標，只回傳最後 `days` 筆與最新一日的訊號（含 52 週新高新低）。
fn build_response(
    symbol: String,
    days: u8,
    bars: &[PriceBar],
    adjustments: &[PriceAdjustment],
) -> TechnicalIndicatorsResponse {
    let bars = technical::adjust_bars(bars, adjustments);
    let bars = bars.as_slice();
    let points = technical::compute(bars);
    let signals = match points.split_last() {
        Some((last, rest)) => technical::signals(rest.last(), last)
            .into_iter()
            .chain(technical::fifty_two_week_signals(bars))
            .map(|signal| TechnicalSignalItem {
                indicator: signal.indicator.to_owned(),
                kind: signal.kind.as_str().to_owned(),
                description: signal.description,
            })
            .collect(),
        None => Vec::new(),
    };
    let start = points.len().saturating_sub(usize::from(days));
    TechnicalIndicatorsResponse {
        data_as_of: points.last().map(|point| point.date.to_string()),
        points: points[start..].iter().map(to_point).collect(),
        stock_symbol: symbol,
        days,
        adjusted: true,
        signals,
    }
}

/// 轉成對外的單日指標，數值四捨五入到小數兩位。
fn to_point(point: &IndicatorPoint) -> TechnicalPoint {
    let ma = |period| point.moving_average(period).map(round2);
    let bias = |period| point.bias(period).map(round2);
    TechnicalPoint {
        date: point.date.to_string(),
        close: round2(point.close),
        ma5: ma(5),
        ma10: ma(10),
        ma20: ma(20),
        ma60: ma(60),
        ma120: ma(120),
        ma240: ma(240),
        bias5: bias(5),
        bias20: bias(20),
        bias60: bias(60),
        bias240: bias(240),
        rsi6: point.rsi6.map(round2),
        rsi12: point.rsi12.map(round2),
        rsi14: point.rsi14.map(round2),
        k: point.k.map(round2),
        d: point.d.map(round2),
        macd_dif: point.macd_dif.map(round2),
        macd_signal: point.macd_signal.map(round2),
        macd_histogram: point.macd_histogram.map(round2),
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn bars(count: usize) -> Vec<PriceBar> {
        (0..count)
            .map(|i| {
                let close = 100.0 + i as f64;
                PriceBar {
                    date: NaiveDate::from_ymd_opt(2026, 1, 1).expect("有效日期")
                        + chrono::Days::new(i as u64),
                    close,
                    high: close + 1.0,
                    low: close - 1.0,
                }
            })
            .collect()
    }

    /// 只回傳最後 days 筆，資料日為最新一筆；持續上漲時 K 值進入超買。
    #[test]
    fn build_response_keeps_last_days_and_latest_signals() {
        let response = build_response("2330".to_owned(), 3, &bars(30), &[]);
        assert_eq!(response.points.len(), 3);
        assert_eq!(response.data_as_of.as_deref(), Some("2026-01-30"));
        assert_eq!(response.points[0].date, "2026-01-28");
        assert!(response.adjusted);
        let last = response.points.last().expect("有最新一筆");
        assert_eq!(last.close, 129.0);
        assert_eq!(last.ma5, Some(127.0));
        assert_eq!(last.bias5, Some(1.57));
        assert!(last.ma60.is_none() && last.bias240.is_none());
        assert_eq!(last.rsi14, Some(100.0));
        assert!(last.macd_dif.is_some() && last.macd_signal.is_none());
        assert!(
            response
                .signals
                .iter()
                .any(|signal| signal.indicator == "kd" && signal.kind == "overbought")
        );
    }

    /// 持續上漲滿 200 個交易日後，最新一日的最高價也是 52 週新高。
    #[test]
    fn build_response_includes_fifty_two_week_signals() {
        let response = build_response("2330".to_owned(), 1, &bars(250), &[]);
        assert!(
            response
                .signals
                .iter()
                .any(|signal| signal.indicator == "52_week" && signal.kind == "new_high"),
            "{:?}",
            response.signals
        );
    }

    /// 除權息事件之前的價格依參考價還原，最新一日維持實際價格。
    #[test]
    fn build_response_adjusts_prices_before_ex_dividend() {
        let raw = bars(30);
        let ex_date = raw[28].date;
        let dividend = PriceAdjustment {
            date: ex_date,
            cash_dividend: 12.7,
            stock_dividend: 0.0,
            share_ratio: 1.0,
        };
        let response = build_response("2330".to_owned(), 3, &raw, &[dividend]);
        let closes: Vec<f64> = response.points.iter().map(|point| point.close).collect();
        // 前收 127、除息 12.7 → 因子 0.9：127 → 114.3；除息日 128 與最新一日 129 不變。
        assert_eq!(closes, [114.3, 128.0, 129.0]);
    }

    /// 日線少於 days 時全部回傳；沒有日線時資料日為 null、沒有訊號。
    #[test]
    fn build_response_handles_short_and_empty_history() {
        assert_eq!(
            build_response("2330".to_owned(), 20, &bars(2), &[])
                .points
                .len(),
            2
        );
        let empty = build_response("2330".to_owned(), 20, &[], &[]);
        assert!(empty.data_as_of.is_none());
        assert!(empty.points.is_empty() && empty.signals.is_empty());
    }
}
