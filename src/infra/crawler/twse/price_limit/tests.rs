//! 上市股價升降幅度（每日漲跌停價）的測試。

use super::*;
use crate::infra::cache::PriceLimit;

fn response(stat: &str, date: &str, rows: serde_json::Value) -> Response {
    serde_json::from_value(serde_json::json!({
        "stat": stat,
        "date": date,
        "fields": ["證券代號","證券名稱","漲停價","開盤競價基準","跌停價","開盤競價基準","收盤價"],
        "data": rows,
    }))
    .expect("測試回應格式正確")
}

/// 一般股票是區間、9,999.95／0.01 是無限制；千分位逗號要去掉，壞列略過。
#[test]
fn parses_ranges_unlimited_and_skips_bad_rows() {
    let parsed = parse(response(
        "OK",
        "20261106",
        serde_json::json!([
            ["1718", "中纖", "10.75", "9.78", "8.81", "9.78", "9.29"],
            [
                "00954",
                "中信日經高股息",
                "9,999.95",
                "14.21",
                "0.01",
                "14.21",
                "13.10"
            ],
            [
                "2330",
                "台積電",
                "1,155.00",
                "1,050.00",
                "945.00",
                "1,050.00",
                "1,000.00"
            ],
            ["9999", "壞列", "--", "1", "--", "1", "1"],
            ["", "沒代號", "1", "1", "1", "1", "1"]
        ]),
    ))
    .expect("格式正確要解析成功");

    assert_eq!(
        parsed.date,
        NaiveDate::from_ymd_opt(2026, 11, 6).expect("valid date")
    );
    assert_eq!(
        parsed.limits,
        vec![
            (
                "1718".to_owned(),
                PriceLimit::Range {
                    down: "8.81".parse().expect("decimal"),
                    up: "10.75".parse().expect("decimal"),
                }
            ),
            ("00954".to_owned(), PriceLimit::Unlimited),
            (
                "2330".to_owned(),
                PriceLimit::Range {
                    down: "945.00".parse().expect("decimal"),
                    up: "1155.00".parse().expect("decimal"),
                }
            ),
        ]
    );
}

/// 查無資料（stat 不是 OK）、日期壞掉、缺欄位都要回錯誤，不能當成「沒有限制」。
#[test]
fn rejects_bad_responses() {
    let empty = serde_json::json!([]);
    let message = parse(response("很抱歉，沒有符合條件的資料!", "", empty.clone()))
        .expect_err("stat 不是 OK 要回錯誤")
        .to_string();
    assert!(message.contains("stat="), "{message}");
    assert!(parse(response("OK", "2026-11-06", empty.clone())).is_err());

    let missing: Response = serde_json::from_value(serde_json::json!({
        "stat": "OK", "date": "20261106", "fields": ["證券代號", "漲停價"], "data": []
    }))
    .expect("測試回應格式正確");
    let message = parse(missing).expect_err("缺跌停價欄位").to_string();
    assert!(message.contains("跌停價"), "{message}");
}

/// visit 打正式網址的路徑與參數（mock，不連外）。
#[tokio::test]
async fn visit_requests_twt84u_without_warrants() {
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rwd/zh/variation/TWT84U"))
        .and(query_param("selectType", "ALLBUT0999"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "stat": "OK", "date": "20261106",
            "fields": ["證券代號","證券名稱","漲停價","開盤競價基準","跌停價"],
            "data": [["00735", "國泰臺韓科技", "9,999.95", "87.15", "0.01"]]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let parsed = crate::core::util::http::mock_origin::with_origins(
        &[("https://www.twse.com.tw", &server.uri())],
        visit(),
    )
    .await
    .expect("mock 回應要解析成功");
    assert_eq!(
        parsed.limits,
        vec![("00735".to_owned(), PriceLimit::Unlimited)]
    );
}
