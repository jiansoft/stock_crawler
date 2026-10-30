//! Winvest 報價爬蟲的單元測試。

use super::*;

const URL: &str = "https://example.test/QueryRecentDailyPrice";

/// 取自 2026-09-29 實際回應（`StockListPrice` 截短）。
const OFFICIAL_BODY: &str = r#"{
    "StockListPrice": [["KlineDatetime", "ClosePrice"], ["09/24", "2475"], ["09/29", "2475"]],
    "StockLastKline": {
        "ChangeRate": 0, "SymbolName": null, "YesterdayClosePrice": 2475,
        "SymbolCode": "2330", "KlinePeriod": 0, "KlineDatetime": "2026-09-29T13:30:00",
        "OpenPrice": 2475, "HighPrice": 2495, "LowPrice": 2475, "ClosePrice": 2475,
        "TransVolume": 24536, "Change": 0
    },
    "DataDate": "2026/09/29", "errMsg": null, "actionUrl": "/Stock/Symbol/QueryRecentDailyPrice"
}"#;

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
}

/// 驗證實際回應的 serde 欄位對應（PascalCase rename、`errMsg` 為 null）。
#[test]
fn response_deserializes_official_shape() {
    let response = parse_query_recent_daily_price(URL, OFFICIAL_BODY).unwrap();
    let kline = response.stock_last_kline.unwrap();

    assert_eq!(kline.close_price, 2475.0);
    assert_eq!(kline.change, 0.0);
    assert_eq!(kline.yesterday_close_price, Some(2475.0));
    assert_eq!(kline.date(), Some(date(2026, 9, 29)));

    // 欄位缺席時（errMsg 有 default）不得反序列化失敗。
    let minimal = parse_query_recent_daily_price(URL, r#"{}"#).unwrap();
    assert!(minimal.stock_last_kline.is_none());
}

#[test]
fn today_kline_accepts_same_day() {
    let response = parse_query_recent_daily_price(URL, OFFICIAL_BODY).unwrap();
    let kline = today_kline(response, "2330", date(2026, 9, 29)).unwrap();

    assert_eq!(kline.close_price, 2475.0);
}

/// 盤中若拿到前一交易日的日 K，不可當成最新成交價。
#[test]
fn today_kline_rejects_previous_trading_day() {
    let response = parse_query_recent_daily_price(URL, OFFICIAL_BODY).unwrap();
    let err = today_kline(response, "2330", date(2026, 9, 30))
        .expect_err("previous day's kline should be rejected");

    assert!(err.to_string().contains("is not today's"));
}

#[test]
fn today_kline_rejects_missing_kline_or_datetime() {
    let empty = parse_query_recent_daily_price(URL, r#"{}"#).unwrap();
    assert!(today_kline(empty, "2330", date(2026, 9, 29)).is_err());

    let body = r#"{ "StockLastKline": { "ClosePrice": 10.0, "Change": 0.0 } }"#;
    let no_datetime = parse_query_recent_daily_price(URL, body).unwrap();
    assert!(today_kline(no_datetime, "2330", date(2026, 9, 29)).is_err());
}

/// token 失效時伺服器回 HTTP 400 空內容，要能辨識出來改換 token 重試。
#[test]
fn looks_like_json_detects_rejected_request() {
    assert!(looks_like_json(OFFICIAL_BODY));
    assert!(!looks_like_json(""));
    assert!(!looks_like_json("<html>400</html>"));
}

#[test]
/// 驗證當 API 的 `ChangeRate` 為 0 時，會用昨收回推並四捨五入到小數第 2 位。
fn test_compute_change_range_fallback_by_yesterday_close() {
    let change_range = compute_change_range(-20.0, Some(1900.0), Some(0.0));
    assert_eq!(change_range, -1.05);
}

/// `ChangeRate` 有效時直接採用，不做回推。
#[test]
fn compute_change_range_prefers_valid_api_rate() {
    assert_eq!(
        compute_change_range(-15.0, Some(1900.0), Some(-0.79)),
        -0.79
    );
    // 平盤（change 為 0）時，rate 為 0 是合法值而非異常，應原樣採用。
    assert_eq!(compute_change_range(0.0, Some(1900.0), Some(0.0)), 0.0);
}

/// API 未提供 `ChangeRate` 時，用昨收回推。
#[test]
fn compute_change_range_computes_from_yesterday_close_when_rate_missing() {
    assert_eq!(compute_change_range(19.0, Some(1900.0), None), 1.0);
}

/// 昨收缺失或為 0 時不可除以零，一律回 0。
#[test]
fn compute_change_range_returns_zero_without_usable_yesterday_close() {
    assert_eq!(compute_change_range(-20.0, None, None), 0.0);
    assert_eq!(compute_change_range(-20.0, Some(0.0), Some(0.0)), 0.0);
    assert_eq!(compute_change_range(-20.0, None, Some(0.0)), 0.0);
}

/// `ChangeRate` 為 NaN 之類的非有限值時，要退回昨收回推而不是把 NaN 傳出去。
#[test]
fn compute_change_range_falls_back_when_rate_is_not_finite() {
    assert_eq!(
        compute_change_range(-20.0, Some(1900.0), Some(f64::NAN)),
        -1.05
    );
}

/// 回應為非 JSON（例如被導到錯誤頁）時，錯誤訊息要帶上內容預覽方便排查。
#[test]
fn parse_rejects_non_json_body() {
    let err = parse_query_recent_daily_price(URL, "<html>503</html>")
        .expect_err("non-JSON body should be an error");

    assert!(
        err.to_string()
            .contains("Failed to parse QueryRecentDailyPrice")
    );
    assert!(err.to_string().contains("body preview: <html>503</html>"));
}

/// API 明確回報 `errMsg` 時必須報錯，不能把空報價當成正常結果。
#[test]
fn parse_rejects_non_empty_err_msg() {
    let body = r#"{ "StockLastKline": null, "errMsg": " 查無此代碼 " }"#;
    let err = parse_query_recent_daily_price(URL, body).expect_err("errMsg should be an error");

    assert!(err.to_string().contains("errMsg is 查無此代碼"));
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
/// 驗證可取得 antiforgery token 並查到最新日 K（不檢查日期，非交易日也能跑）。
async fn test_fetch_data() {
    dotenvy::dotenv().ok();

    let response = fetch_data("2330").await.expect("fetch_data");
    let kline = response.stock_last_kline.expect("StockLastKline");
    println!(
        "winvest 2330: datetime={:?} close={} change={}",
        kline.kline_datetime, kline.close_price, kline.change
    );
    assert!(kline.close_price > 0.0);
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
/// 驗證 Winvest 可取得統一格式報價資訊（最新日 K 不是今天時會回錯誤）。
async fn test_get_stock_quotes() {
    dotenvy::dotenv().ok();

    match Winvest::get_stock_quotes("2330").await {
        Ok(quotes) => println!("winvest quotes: {:?}", quotes),
        Err(why) => println!("Failed to winvest::get_stock_quotes because {:?}", why),
    }
}
