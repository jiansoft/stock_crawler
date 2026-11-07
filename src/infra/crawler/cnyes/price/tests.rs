//! 鉅亨網價格爬蟲的測試；連外的測試標 #[ignore]。

use crate::infra::crawler::log_stock_price_test;

use super::*;

/// 驗證正常回應的欄位對應：鉅亨的欄位名是數字代碼
/// （"6"=成交價、"11"=漲跌、"56"=漲跌幅），serde rename 不能失效。
#[test]
fn test_deserialize_response_maps_numeric_field_names() {
    let body = r#"{
        "statusCode": 200,
        "message": "OK",
        "data": [{
            "6": 1435.0,
            "11": -15.0,
            "56": -1.03,
            "200007": "台積電"
        }]
    }"#;
    let response: Response = serde_json::from_str(body).expect("response should deserialize");
    let quote = response.data.first().expect("expected one quote row");

    assert_eq!(quote.required_current_price("2330").unwrap(), 1435.0);
    assert_eq!(quote.change_or_zero(), -15.0);
    assert_eq!(quote.change_range_or_zero(), -1.03);
}

#[test]
fn test_deserialize_response_with_null_current_price() {
    let body = r#"{
        "statusCode": 200,
        "message": "OK",
        "data": [{
            "6": null,
            "11": 0.5,
            "56": 1.2
        }]
    }"#;
    let response: Response = serde_json::from_str(body).expect("response should deserialize");
    let quote = response.data.first().expect("expected one quote row");
    let err = quote
        .required_current_price("5306")
        .expect_err("null current price should return error");

    assert!(err.to_string().contains("field `6`"));
    assert!(err.to_string().contains("5306"));
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_get_stock_price() {
    dotenvy::dotenv().ok();
    log_stock_price_test::<CnYes>("2330").await;
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_get_stock_quotes() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 cnyes::get_stock_quotes");

    match CnYes::get_stock_quotes("2330").await {
        Ok(e) => {
            dbg!(&e);
            tracing::debug!("cnyes::get_stock_quotes : {:#?}", e);
        }
        Err(why) => {
            tracing::debug!("Failed to cnyes::get_stock_quotes because {:?}", why);
        }
    }

    tracing::debug!("結束 cnyes::get_stock_quotes");
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_fetch_data() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_data");

    // match get("2330").await {
    match fetch_data("2330").await {
        Ok(e) => {
            dbg!(&e);
            tracing::debug!("price : {:#?}", e);
        }
        Err(why) => {
            tracing::debug!("Failed to fetch_data because {:?}", why);
        }
    }

    tracing::debug!("結束 fetch_data");
}

/// 以本機 mock server 取代鉅亨（C18，沿用 G15 的 mock_origin）：正常報價、成交價為 null
/// 與沒有資料列三種回應。
#[tokio::test]
async fn quotes_from_mock_server() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::core::util::http::mock_origin::with_origins;

    let server = MockServer::start().await;
    let respond = |symbol: &str, body: serde_json::Value| {
        Mock::given(method("GET"))
            .and(path(format!("/ws/api/v1/quote/quotes/TWS:{symbol}:STOCK")))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
    };
    respond(
        "2330",
        serde_json::json!({"statusCode": 200, "message": "OK", "data": [{"6": 1435.0, "56": -1.03}]}),
    )
    .mount(&server)
    .await;
    respond(
        "5306",
        serde_json::json!({"statusCode": 200, "message": "OK", "data": [{"6": null}]}),
    )
    .mount(&server)
    .await;
    respond(
        "9999",
        serde_json::json!({"statusCode": 200, "message": "OK", "data": []}),
    )
    .mount(&server)
    .await;

    let (price, quotes, null_price, empty) =
        with_origins(&[("https://ws.api.cnyes.com", &server.uri())], async {
            (
                CnYes::get_stock_price("2330").await,
                CnYes::get_stock_quotes("2330").await,
                CnYes::get_stock_quotes("5306").await,
                CnYes::get_stock_price("9999").await,
            )
        })
        .await;

    assert_eq!(
        price.expect("有成交價"),
        Decimal::try_from(1435.0).expect("有效數值")
    );
    let quotes = quotes.expect("有報價");
    assert_eq!(
        (quotes.price, quotes.change, quotes.change_range),
        (1435.0, 0.0, -1.03),
        "漲跌缺值時退回 0"
    );
    assert!(null_price.is_err(), "成交價為 null");
    let empty = empty.expect_err("沒有資料列");
    assert!(empty.to_string().contains("data is empty"), "{empty:#}");
}
