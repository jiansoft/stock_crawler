//! 條件選股回測 endpoint 整合測試：均值回歸（年線乖離率上限與依乖離排序），只用換股當天的日線。

use super::*;

/// 兩個換股日（03-01、03-08）的收盤與年線：BA 乖離 -20%、BB -5%、BC +10%；BD 沒有年線（上市未滿一年）。
async fn seed_bias(pool: &sqlx::PgPool) {
    sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979B_'"#)
        .execute(pool)
        .await
        .expect("清除日線");
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "MovingAverage240")
           SELECT d::date, s, close, ma
           FROM (VALUES ('79979BA', 80, 100), ('79979BB', 95, 100), ('79979BC', 110, 100)) AS q(s, close, ma)
           CROSS JOIN (VALUES ('1993-03-01'), ('1993-03-08')) AS t(d)"#,
    )
    .execute(pool)
    .await
    .expect("種入收盤與年線");
}

/// `max_bias240=-10` 只留 BA；`max_bias240=0` 留 BA、BB；依乖離排序兩期都是 BA、BB、BC（與預設依相對強度不同）；
/// 超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_filters_and_ranks_by_bias240() {
    let Some(pool) = seeded("screen_backtest_filters_and_ranks_by_bias240").await else {
        return;
    };
    seed_bias(pool).await;

    let (status, json) = query("limit=3&max_bias240=-10").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["max_bias240"], -10.0);
    let deep = vec!["79979BA".to_owned()];
    assert_eq!(holdings(&json), [deep.clone(), deep]);

    let (status, json) = query("limit=3&max_bias240=0").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let below = vec!["79979BA".to_owned(), "79979BB".to_owned()];
    assert_eq!(holdings(&json), [below.clone(), below]);

    let (status, json) = query("limit=3&rank_by=bias240").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["rank_by"], "bias240");
    for period in json["periods"].as_array().expect("periods") {
        assert_eq!(
            period["symbols"],
            serde_json::json!(["79979BA", "79979BB", "79979BC"]),
            "{period}"
        );
    }

    for extra in ["max_bias240=-91", "max_bias240=101"] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
    }

    cleanup(pool).await;
}
