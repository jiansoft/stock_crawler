//! 元大價格爬蟲的測試；連外的測試標 #[ignore]。

use super::*;

/// 樣本取自元大 API 的實際回應形狀（僅保留本模組會用到的欄位）。
const SUCCESS_BODY: &str = r#"{
    "status": 0,
    "data": {
        "deal": 1435.0,
        "trend": -15.0,
        "trendPercentage": -1.03,
        "name": "台積電"
    }
}"#;

/// 網址一旦改錯就會整個來源失效，這裡把格式鎖定住。
#[test]
fn build_url_points_to_currentstock_endpoint() {
    assert_eq!(
        build_url("2330"),
        "https://ytdf.yuanta.com.tw/prod/yesidmz/api/basic/currentstock?symbol=2330"
    );
}

/// 驗證 serde 欄位對應：`trendPercentage` 需 rename，未列出的欄位要被忽略。
#[test]
fn response_deserializes_official_shape() {
    let response: Response = serde_json::from_str(SUCCESS_BODY).unwrap();

    assert_eq!(response.status, 0);
    assert_eq!(response.data.deal, 1435.0);
    assert_eq!(response.data.trend, -15.0);
    assert_eq!(response.data.trend_percentage, -1.03);
}

/// `status` 為 0 時應原樣取出報價資料。
#[test]
fn parse_response_returns_data_when_status_is_zero() {
    let response: Response = serde_json::from_str(SUCCESS_BODY).unwrap();
    let data = parse_response("https://example.test", response).unwrap();

    assert_eq!(data.deal, 1435.0);
    assert_eq!(data.trend, -15.0);
    assert_eq!(data.trend_percentage, -1.03);
}

/// `status` 非 0 代表來源回報失敗，必須報錯而不是回傳資料。
#[test]
fn parse_response_rejects_non_zero_status() {
    let body =
        r#"{ "status": 999, "data": { "deal": 0.0, "trend": 0.0, "trendPercentage": 0.0 } }"#;
    let response: Response = serde_json::from_str(body).unwrap();
    let err = parse_response("https://example.test/quote", response)
        .expect_err("non-zero status should be an error");

    assert!(err.to_string().contains("status is 999"));
    assert!(err.to_string().contains("https://example.test/quote"));
}

/// 以本機 mock server 取代元大即時報價 API（G15），驗證 `StockInfo` 兩個入口（C19）。
mod mock_sources {
    use rust_decimal_macros::dec;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;

    const QUOTE_PATH: &str = "/prod/yesidmz/api/basic/currentstock";

    async fn server_with(body: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(QUOTE_PATH))
            .and(query_param("symbol", "2330"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;
        server
    }

    /// 成交價轉成 `Decimal`。
    #[tokio::test]
    async fn get_stock_price_returns_deal() {
        let server = server_with(SUCCESS_BODY).await;
        let uri = server.uri();

        let price = with_origins(
            &[("https://ytdf.yuanta.com.tw", &uri)],
            Yuanta::get_stock_price("2330"),
        )
        .await
        .expect("成功回應應取得成交價");

        assert_eq!(price, dec!(1435));
    }

    /// 成交價、漲跌與漲跌幅都帶進統一報價格式。
    #[tokio::test]
    async fn get_stock_quotes_maps_all_fields() {
        let server = server_with(SUCCESS_BODY).await;
        let uri = server.uri();

        let quotes = with_origins(
            &[("https://ytdf.yuanta.com.tw", &uri)],
            Yuanta::get_stock_quotes("2330"),
        )
        .await
        .expect("成功回應應取得報價");

        assert_eq!(quotes.stock_symbol, "2330");
        assert_eq!(quotes.price, 1435.0);
        assert_eq!(quotes.change, -15.0);
        assert_eq!(quotes.change_range, -1.03);
    }

    /// 來源回報 `status` 非 0 時兩個入口都回錯誤，不回傳 0 元的假價格。
    #[tokio::test]
    async fn non_zero_status_is_an_error_for_both_entries() {
        let body =
            r#"{ "status": 1, "data": { "deal": 0.0, "trend": 0.0, "trendPercentage": 0.0 } }"#;
        let price_server = server_with(body).await;
        let quotes_server = server_with(body).await;
        let price_uri = price_server.uri();
        let quotes_uri = quotes_server.uri();

        let price = with_origins(
            &[("https://ytdf.yuanta.com.tw", &price_uri)],
            Yuanta::get_stock_price("2330"),
        )
        .await;
        let quotes = with_origins(
            &[("https://ytdf.yuanta.com.tw", &quotes_uri)],
            Yuanta::get_stock_quotes("2330"),
        )
        .await;

        assert!(price.is_err(), "{price:?}");
        assert!(quotes.is_err(), "{quotes:?}");
    }
}

#[tokio::test]
#[ignore]
async fn test_get_stock_price() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 yuanta::get_stock_price");

    for stock_symbol in ["2330", "4536"] {
        match Yuanta::get_stock_price(stock_symbol).await {
            Ok(price) => {
                tracing::debug!("yuanta {stock_symbol} price: {price}")
            }
            Err(why) => tracing::debug!(
                "Failed to yuanta::get_stock_price({stock_symbol}) because {:?}",
                why
            ),
        }
    }

    tracing::debug!("結束 yuanta::get_stock_price");
}

#[tokio::test]
#[ignore]
async fn test_get_stock_quotes() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 yuanta::get_stock_quotes");

    for stock_symbol in ["2330", "5306"] {
        match Yuanta::get_stock_quotes(stock_symbol).await {
            Ok(quotes) => {
                tracing::debug!("yuanta::get_stock_quotes {stock_symbol}: {:?}", quotes)
            }
            Err(why) => tracing::debug!(
                "Failed to yuanta::get_stock_quotes({stock_symbol}) because {:?}",
                why
            ),
        }
    }

    tracing::debug!("結束 yuanta::get_stock_quotes");
}
