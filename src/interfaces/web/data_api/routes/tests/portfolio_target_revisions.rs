//! 目標配置歷史版本端點的隔離資料庫語意：列出、還原、參數驗證與跨會員隔離。
use super::support::{get, pool_or_skip};
use super::*;
use serde_json::{Value, json};

/// 帶同一組測試 Bearer 金鑰送出 JSON 請求（PUT 目標配置或 POST 還原）。
async fn send(method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let key = std::env::var("DATA_API_KEY").expect("先以 get 建立測試金鑰");
    let response = router()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Authorization", format!("Bearer {key}"))
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1_048_576)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

async fn put_targets(targets: Value) -> (StatusCode, Value) {
    send(
        "PUT",
        "/api/v1/portfolio/targets",
        json!({"member_id": 79977, "dimension": "industry", "targets": targets}),
    )
    .await
}

/// 儲存兩次後有一版；還原後內容回到那一版，還原前的設定又多記一版；別的會員還原不到。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn revisions_list_and_restore_previous_targets() {
    let pool = pool_or_skip("revisions_list_and_restore_previous_targets")
        .await
        .expect("隔離 DB 必須可用");
    for sql in [
        "DELETE FROM portfolio_target WHERE member_id='79977'",
        "DELETE FROM portfolio_target_dimension WHERE member_id='79977'",
        "DELETE FROM portfolio_target_revision WHERE member_id='79977'",
    ] {
        sqlx::query(sql).execute(pool).await.unwrap();
    }
    let list = "/api/v1/portfolio/targets/revisions?member_id=79977&dimension=industry";
    for query in [
        "member_id=-1&dimension=stock",
        "dimension=bad",
        "member_id=79977",
    ] {
        let (status, body) = get(&format!("/api/v1/portfolio/targets/revisions?{query}")).await;
        assert!(status.is_client_error(), "{query}: {status} {body}");
    }
    let (status, body) = get(list).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revisions"], json!([]));

    let first = json!([{"target_key": "半導體業", "target_percent": 60}, {"target_key": "金融保險業", "target_percent": 15.5}]);
    assert_eq!(put_targets(first).await.0, StatusCode::OK);
    assert_eq!(
        put_targets(json!([{"target_key": "航運業", "target_percent": 30}]))
            .await
            .0,
        StatusCode::OK
    );
    let (_, body) = get(list).await;
    let revisions = body["revisions"].as_array().unwrap();
    assert_eq!(revisions.len(), 1, "{body}");
    assert_eq!(revisions[0]["target_cash_percent"], json!(24.5));
    assert_eq!(
        revisions[0]["targets"],
        json!([{"target_key": "半導體業", "target_percent": 60.0}, {"target_key": "金融保險業", "target_percent": 15.5}])
    );
    let id = revisions[0]["id"].as_i64().unwrap();

    let restore = format!("/api/v1/portfolio/targets/revisions/{id}/restore");
    let (status, _) = send("POST", &restore, json!({"member_id": -1})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = send("POST", &restore, json!({"member_id": 79976})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, restored) = send("POST", &restore, json!({"member_id": 79977})).await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(restored["dimension"], "industry");
    assert_eq!(restored["target_cash_percent"], json!(24.5));
    assert_eq!(restored["targets"].as_array().unwrap().len(), 2);

    // 還原前的「航運業 30%」記成最新的一版，可以再還原回去。
    let (_, body) = get(list).await;
    let revisions = body["revisions"].as_array().unwrap();
    assert_eq!(revisions.len(), 2, "{body}");
    assert_eq!(
        revisions[0]["targets"],
        json!([{"target_key": "航運業", "target_percent": 30.0}])
    );

    for sql in [
        "DELETE FROM portfolio_target WHERE member_id='79977'",
        "DELETE FROM portfolio_target_dimension WHERE member_id='79977'",
        "DELETE FROM portfolio_target_revision WHERE member_id='79977'",
    ] {
        sqlx::query(sql).execute(pool).await.unwrap();
    }
}
