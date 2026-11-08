//! 上櫃次日漲跌停價的測試。

use super::*;
use crate::infra::cache::PriceLimit;

fn body(date: &str, rows: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "tables": [{
            "title": "上櫃股票每日收盤行情(不含定價)",
            "date": date,
            "fields": ["代號","名稱","收盤 ","漲跌","次日漲停價 ","次日跌停價"],
            "data": rows,
        }]
    })
}

/// 欄位名稱尾端的空白要忽略；債券 ETF 是無限制、一般股票是區間。
#[test]
fn parses_next_day_limits() {
    let response: Response = serde_json::from_value(body(
        "115/11/06",
        serde_json::json!([
            [
                "00679B",
                "元大美債20年",
                "26.89",
                "-0.10",
                "9,999.95",
                "0.01"
            ],
            ["6488", "環球晶", "512.00", "-10.00", "563.00", "461.00"],
            ["8888", "壞列", "1", "0", "--", "--"]
        ]),
    ))
    .expect("測試回應格式正確");
    let parsed = parse(response).expect("格式正確要解析成功");

    assert_eq!(
        parsed.date,
        NaiveDate::from_ymd_opt(2026, 11, 6).expect("valid date")
    );
    assert_eq!(
        parsed.limits,
        vec![
            ("00679B".to_owned(), PriceLimit::Unlimited),
            (
                "6488".to_owned(),
                PriceLimit::Range {
                    down: "461.00".parse().expect("decimal"),
                    up: "563.00".parse().expect("decimal"),
                }
            ),
        ]
    );
}

/// 沒有資料表、日期壞掉、缺欄位都要回錯誤。
#[test]
fn rejects_bad_responses() {
    let empty: Response =
        serde_json::from_value(serde_json::json!({ "tables": [] })).expect("測試回應格式正確");
    assert!(parse(empty).is_err());

    let bad_date: Response = serde_json::from_value(body("2026-11-06", serde_json::json!([])))
        .expect("測試回應格式正確");
    assert!(parse(bad_date).is_err());

    let missing: Response = serde_json::from_value(serde_json::json!({
        "tables": [{ "date": "115/11/06", "fields": ["代號", "次日漲停價"], "data": [] }]
    }))
    .expect("測試回應格式正確");
    let message = parse(missing).expect_err("缺次日跌停價").to_string();
    assert!(message.contains("次日跌停價"), "{message}");

    assert_eq!(parse_roc_date("115/11/06/1"), None);
    assert_eq!(parse_roc_date("115/13/06"), None);
}

/// visit 打正式網址的路徑與參數（mock，不連外）。
#[tokio::test]
async fn visit_requests_otc_close_quotes() {
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/www/zh-tw/afterTrading/otc"))
        .and(query_param("type", "EW"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body(
            "115/11/06",
            serde_json::json!([[
                "00937B",
                "群益ESG投等債20+",
                "14.47",
                "0",
                "9,999.95",
                "0.01"
            ]]),
        )))
        .expect(1)
        .mount(&server)
        .await;

    let parsed = util::http::mock_origin::with_origins(
        &[("https://www.tpex.org.tw", &server.uri())],
        visit(),
    )
    .await
    .expect("mock 回應要解析成功");
    assert_eq!(
        parsed.limits,
        vec![("00937B".to_owned(), PriceLimit::Unlimited)]
    );
}
