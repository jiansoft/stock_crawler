//! 快取回應、判斷規則與 Redis 讀取的驗證。

use super::*;
use chrono::NaiveDate;

/// 固定的策略優勢快取，包含可判斷與無資料的情況。
fn record(median: Option<f64>, beat_rate: Option<f64>) -> EdgeRecord {
    let date = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
    EdgeRecord {
        name: "最佳組合".to_owned(),
        checked: date,
        recent: EdgeWindow {
            from: date,
            to: date,
            offsets: 20,
            median_return: median,
            etf_median_return: Some(59.78),
            beat_rate,
        },
        full: None,
    }
}

/// 讀取回應 JSON，確認數字、null 與判斷欄位沒有改變。
async fn body(response: Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// 成功、轉弱、缺資料、空清單及損壞各有固定回應。
#[tokio::test]
async fn response_contract_and_errors() {
    for (median, beat, verdict) in [
        (Some(66.56), Some(0.75), "holds"),
        (Some(50.0), Some(0.4), "weakened"),
        (None, None, "unknown"),
    ] {
        let json = serde_json::to_string(&vec![record(median, beat)]).unwrap();
        let response = decode_response(&json);
        assert_eq!(response.status(), StatusCode::OK);
        let json = body(response).await;
        assert_eq!(json["strategies"][0]["verdict"], verdict);
        assert_eq!(
            json["strategies"][0]["recent"]["median_return"],
            serde_json::json!(median)
        );
        assert!(json["strategies"][0]["full"].is_null());
        assert_eq!(json["execution_price"], "next_open");
        assert_eq!(json["slippage_bps"], 20);
        assert!(
            json["disclaimer"]
                .as_str()
                .unwrap()
                .contains("亦不會自動下單")
        );
    }
    assert_eq!(decode_response("[]").status(), StatusCode::NOT_FOUND);
    let response = decode_response("secret invalid json");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body(response).await.to_string().contains("secret"));
}

/// CI 的 Redis 驗證使用獨立鍵；本機 Redis 不可用時略過。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要 CI Redis")]
async fn redis_found_missing_and_corrupt() {
    if redis::CLIENT.ping().await.is_err() {
        return;
    }
    let key = "strategy_edge:test:data-api:79979";
    redis::CLIENT.delete(key).await.unwrap();
    assert_eq!(load_response(key).await.status(), StatusCode::NOT_FOUND);
    redis::CLIENT
        .set(
            key,
            serde_json::to_string(&vec![record(Some(66.56), Some(0.75))]).unwrap(),
            60,
        )
        .await
        .unwrap();
    assert_eq!(load_response(key).await.status(), StatusCode::OK);
    redis::CLIENT.set(key, "broken", 60).await.unwrap();
    assert_eq!(
        load_response(key).await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    redis::CLIENT.delete(key).await.unwrap();
}

/// 斷線不可偽裝成沒有檢查紀錄。
#[tokio::test]
async fn redis_connection_error_is_unavailable() {
    let response = cached_response(Err(RedisError::Parse(
        "private connection detail".to_owned(),
    )));
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        !body(response)
            .await
            .to_string()
            .contains("private connection detail")
    );
}
