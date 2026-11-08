//! BigGo 財經的測試。

use super::*;

const URL: &str = "https://api.biggo.com/api/v1/finance/test?region=tw";

/// 成功回 `data`；`data: null` 回 `None`。
#[test]
fn decode_returns_data() {
    let data: Option<i32> =
        decode("/test", URL, StatusCode::OK, r#"{"result":true,"data":7}"#).expect("可解析");
    assert_eq!(data, Some(7));
    let none: Option<i32> = decode(
        "/test",
        URL,
        StatusCode::OK,
        r#"{"result":true,"data":null}"#,
    )
    .expect("可解析");
    assert_eq!(none, None);
}

/// 非 2xx、格式不符、`result=false` 都回錯誤，錯誤訊息裡的 body 是單行片段。
#[test]
fn decode_rejects_failures_with_single_line_body() {
    let cases = [
        (
            StatusCode::BAD_GATEWAY,
            "<html>\n  <head>\n</html>",
            "HTTP 502",
        ),
        (StatusCode::OK, "not\njson", "Error parsing"),
        (
            StatusCode::OK,
            "{\"result\": false,\n \"error\": {\"code\": 1002}}",
            "result=false",
        ),
    ];
    for (status, body, want) in cases {
        let err = decode::<i32>("/test", URL, status, body).expect_err("應回錯誤");
        let message = err.to_string();
        assert!(message.contains(want), "{message}");
        assert!(!message.contains('\n'), "{message}");
    }
}
