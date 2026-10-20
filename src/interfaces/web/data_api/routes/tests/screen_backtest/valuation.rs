//! 條件選股回測 endpoint 整合測試：換股當天的官方本益比與股價淨值比上限。

use super::*;

/// 換股日的本益比／股價淨值比（03-15 沒有日線）：
///
/// | 日 | BA | BB | BC |
/// |---|---|---|---|
/// | 03-01 | 10／1.5 | 0（虧損）／2 | 30／3 |
/// | 03-08 | 12／1.2 | 8／0.8 | 9／5 |
async fn seed_valuations(pool: &sqlx::PgPool) {
    for (date, symbol, pe, pb) in [
        ("1993-03-01", "79979BA", "10", "1.5"),
        ("1993-03-01", "79979BB", "0", "2"),
        ("1993-03-01", "79979BC", "30", "3"),
        ("1993-03-08", "79979BA", "12", "1.2"),
        ("1993-03-08", "79979BB", "8", "0.8"),
        ("1993-03-08", "79979BC", "9", "5"),
    ] {
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "PriceEarningRatio", "price-to-book_ratio")
               VALUES ($1::date, $2, $3::numeric, $4::numeric)
               ON CONFLICT (stock_symbol, "Date") DO UPDATE
               SET "PriceEarningRatio" = EXCLUDED."PriceEarningRatio",
                   "price-to-book_ratio" = EXCLUDED."price-to-book_ratio""#,
        )
        .bind(date)
        .bind(symbol)
        .bind(pe)
        .bind(pb)
        .execute(pool)
        .await
        .expect("種入本益比與股價淨值比");
    }
}

/// `max_pe`／`max_pb` 只看換股當天的值，0（虧損或沒有資料）不會通過；超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_valuation_caps_use_rebalance_day_ratios() {
    let Some(pool) = seeded("screen_backtest_valuation_caps_use_rebalance_day_ratios").await else {
        return;
    };
    seed_valuations(pool).await;

    // 本益比 ≤ 15：03-01 只有 BA（BB 虧損、BC 30 倍）；03-08 依相對強度取 BB、BC（BC 沒有報酬）。
    // 03-15 沒有日線，沒有持股。
    let (status, json) = query("limit=2&max_pe=15").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["max_pe"], 15.0);
    assert!(json["max_pb"].is_null(), "{json}");
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 2, Some(8.0)),
        ]
    );
    assert_eq!(
        json["current"]["holdings"].as_array().map(Vec::len),
        Some(0)
    );

    // 股價淨值比 ≤ 1.6：03-01 只有 BA；03-08 BB、BA（BC 5 倍）。
    let (status, json) = query("limit=2&max_pb=1.6").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["max_pb"], 1.6);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 2, Some(5.5)),
        ]
    );

    // 兩者併用：03-08 的 BB、BA 中本益比也 ≤ 10 的只有 BB。
    let (status, json) = query("limit=2&max_pe=10&max_pb=1.6").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(8.0)),
        ]
    );

    let (_, json) = query("limit=2").await;
    assert!(json["max_pe"].is_null(), "{json}");

    for bad in ["max_pe=-1", "max_pe=10001", "max_pb=1001"] {
        let (status, json) = query(&format!("limit=2&{bad}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {json}");
    }
}
