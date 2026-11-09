//! 條件選股回測 endpoint 整合測試：Minervini 趨勢模板只看換股當天的日線。

use super::*;

/// 換股日的日線 `(日期, 代號, 收盤, MA20, MA60, MA120, MA240, 52 週最高, 52 週最低)`（03-15 沒有日線）：
///
/// | 日 | BA | BB | BC |
/// |---|---|---|---|
/// | 03-01 | 符合 | MA20 < MA60 | 符合 |
/// | 03-08 | 收盤低於高點 75% | 符合 | 52 週最低價為 0 |
async fn seed_trends(pool: &sqlx::PgPool) {
    for (date, symbol, close, ma20, ma60, ma120, ma240, high, low) in [
        ("1993-03-01", "79979BA", 100, 95, 90, 85, 80, 110, 60),
        ("1993-03-01", "79979BB", 100, 85, 90, 85, 80, 110, 60),
        ("1993-03-01", "79979BC", 100, 95, 90, 85, 80, 110, 60),
        ("1993-03-08", "79979BA", 100, 95, 90, 85, 80, 140, 60),
        ("1993-03-08", "79979BB", 100, 95, 90, 85, 80, 110, 60),
        ("1993-03-08", "79979BC", 100, 95, 90, 85, 80, 110, 0),
    ] {
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "MovingAverage20", "MovingAverage60",
                   "MovingAverage120", "MovingAverage240", maximum_price_in_year, minimum_price_in_year)
               VALUES ($1::date, $2, $3, $4, $5, $6, $7, $8, $9)
               ON CONFLICT (stock_symbol, "Date") DO UPDATE
               SET "ClosingPrice" = EXCLUDED."ClosingPrice",
                   "MovingAverage20" = EXCLUDED."MovingAverage20",
                   "MovingAverage60" = EXCLUDED."MovingAverage60",
                   "MovingAverage120" = EXCLUDED."MovingAverage120",
                   "MovingAverage240" = EXCLUDED."MovingAverage240",
                   maximum_price_in_year = EXCLUDED.maximum_price_in_year,
                   minimum_price_in_year = EXCLUDED.minimum_price_in_year"#,
        )
        .bind(date)
        .bind(symbol)
        .bind(Decimal::from(close))
        .bind(Decimal::from(ma20))
        .bind(Decimal::from(ma60))
        .bind(Decimal::from(ma120))
        .bind(Decimal::from(ma240))
        .bind(Decimal::from(high))
        .bind(Decimal::from(low))
        .execute(pool)
        .await
        .expect("種入換股日日線");
    }
}

/// `trend_template=true` 只留換股當天符合趨勢模板的股票；52 週最低價為 0 視為資料不足；回應回填參數。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_trend_template_uses_rebalance_day_quotes() {
    let Some(pool) = seeded("screen_backtest_trend_template_uses_rebalance_day_quotes").await
    else {
        return;
    };
    seed_trends(pool).await;

    // 03-01：BA（10%）、BC（2%）；03-08：只有 BB（8%）。
    let (status, json) = query("limit=2&trend_template=true").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["trend_template"], true);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(6.0)),
            ("1993-03-08".to_owned(), 1, Some(8.0)),
        ]
    );

    let (status, json) = query("limit=2&trend_template=false").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["trend_template"], false);
    cleanup(pool).await;
}
