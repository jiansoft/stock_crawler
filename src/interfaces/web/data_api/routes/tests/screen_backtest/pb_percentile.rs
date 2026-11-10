//! 條件選股回測 endpoint 整合測試：便宜價（股價淨值比近 5 年百分位上限與依分位排序），只用換股當天的分位。

use super::*;

/// 兩個換股日的股價淨值比分位：BA 兩天都是 10；BB 03-01 為 60、03-08 為 30；BC 03-01 為 5、03-08 沒有資料。
async fn seed_pb_percentiles(pool: &sqlx::PgPool) {
    sqlx::query(
        "INSERT INTO daily_pb_percentile (date, stock_symbol, pb, pb_percentile, samples) VALUES
             ('1993-03-01', '79979BA', 1, 10, 1000), ('1993-03-08', '79979BA', 1, 10, 1000),
             ('1993-03-01', '79979BB', 2, 60, 1000), ('1993-03-08', '79979BB', 2, 30, 1000),
             ('1993-03-01', '79979BC', 1, 5, 1000)",
    )
    .execute(pool)
    .await
    .expect("種入淨值比分位");
}

/// `max_pb_percentile=20` 03-01 留 BA、BC，03-08 只留 BA（BC 沒有資料不通過）；`=50` 03-08 再加上 BB；
/// 依分位排序 03-01 為 BC、BA、BB，03-08 為 BA、BB、BC（沒有資料排最後，與預設依相對強度不同）；超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_filters_and_ranks_by_pb_percentile() {
    let Some(pool) = seeded("screen_backtest_filters_and_ranks_by_pb_percentile").await else {
        return;
    };
    seed_pb_percentiles(pool).await;

    let (status, json) = query("limit=3&max_pb_percentile=20").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["max_pb_percentile"], 20.0);
    assert_eq!(
        holdings(&json),
        [
            vec!["79979BA".to_owned(), "79979BC".to_owned()],
            vec!["79979BA".to_owned()],
        ]
    );

    let (status, json) = query("limit=3&max_pb_percentile=50").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(
        holdings(&json),
        [
            vec!["79979BA".to_owned(), "79979BC".to_owned()],
            vec!["79979BA".to_owned(), "79979BB".to_owned()],
        ]
    );

    let (status, json) = query("limit=3&rank_by=pb_percentile").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["rank_by"], "pb_percentile");
    let symbols: Vec<_> = json["periods"]
        .as_array()
        .expect("periods")
        .iter()
        .map(|period| period["symbols"].clone())
        .collect();
    assert_eq!(
        symbols,
        [
            serde_json::json!(["79979BC", "79979BA", "79979BB"]),
            serde_json::json!(["79979BA", "79979BB", "79979BC"]),
        ]
    );

    for extra in ["max_pb_percentile=-1", "max_pb_percentile=101"] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
    }

    cleanup(pool).await;
}
