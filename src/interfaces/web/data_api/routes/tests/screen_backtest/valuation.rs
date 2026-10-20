//! 條件選股回測 endpoint 整合測試：換股當天的官方本益比與股價淨值比上限與排序。

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

    // 依本益比由低到高：03-01 BA（10）、BC（30），BB 虧損排最後；03-08 BB（8）、BC（9）。
    let (status, json) = query("limit=2&rank_by=pe_ratio").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["rank_by"], "pe_ratio");
    assert_eq!(
        periods(&json)[..2],
        [
            ("1993-03-01".to_owned(), 2, Some(6.0)),
            ("1993-03-08".to_owned(), 2, Some(8.0)),
        ]
    );
    // 依股價淨值比由低到高：03-01 BA（1.5）、BB（2）；03-08 BB（0.8）、BA（1.2）。
    let (status, json) = query("limit=2&rank_by=pb_ratio").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(
        periods(&json)[..2],
        [
            ("1993-03-01".to_owned(), 2, Some(3.0)),
            ("1993-03-08".to_owned(), 2, Some(5.5)),
        ]
    );

    let (_, json) = query("limit=2").await;
    assert!(json["max_pe"].is_null(), "{json}");

    // 大盤估值濾網：03-01 市場本益比中位數 12、03-08 為 25；門檻 20 時 03-08 空手，門檻 25 剛好通過。
    // 03-15 沒有估值列，視為通過。
    let dates = [day(1), day(8)];
    let clear = || async {
        let _ = sqlx::query("DELETE FROM daily_market_valuation WHERE date = ANY($1)")
            .bind(dates.to_vec())
            .execute(pool)
            .await;
    };
    clear().await;
    for (date, median_pe) in dates.into_iter().zip([12, 25]) {
        sqlx::query(
            "INSERT INTO daily_market_valuation (date, stock_exchange_market_id, median_pe, pe_samples, pb_samples)
             VALUES ($1, 0, $2, 3, 3)",
        )
        .bind(date)
        .bind(rust_decimal::Decimal::from(median_pe))
        .execute(pool)
        .await
        .expect("種入市場估值");
    }
    let (status, json) = query("limit=2&market_filter_max_pe=20").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["market_filter_max_pe"], 20.0);
    assert_eq!(json["periods"][0]["market_filtered"], false);
    assert_eq!(json["periods"][1]["market_filtered"], true);
    assert_eq!(json["summary"]["market_filtered_periods"], 1);
    let (_, json) = query("limit=2&market_filter_max_pe=25").await;
    assert_eq!(json["summary"]["market_filtered_periods"], 0);
    // 上櫃（市場 4）沒有估值列：都通過。
    let (_, json) = query("market=tpex&market_filter_max_pe=5").await;
    assert_eq!(json["summary"]["market_filtered_periods"], 0);
    clear().await;

    for bad in [
        "max_pe=-1",
        "max_pe=10001",
        "max_pb=1001",
        "market_filter_max_pe=4.9",
        "market_filter_max_pe=101",
    ] {
        let (status, json) = query(&format!("limit=2&{bad}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {json}");
    }
}

/// 相對估值濾網：只用換股日之前近 5 年的市場本益比中位數算百分位；歷史不足 250 天視為通過。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_market_pe_percentile_uses_prior_history_only() {
    let Some(pool) = seeded("screen_backtest_market_pe_percentile_uses_prior_history_only").await
    else {
        return;
    };
    let clear = || async {
        let _ = sqlx::query(
            "DELETE FROM daily_market_valuation
             WHERE stock_exchange_market_id = 0 AND date BETWEEN '1992-01-01' AND '1993-03-31'",
        )
        .execute(pool)
        .await;
    };
    clear().await;
    // 換股日之前的 300 天：中位數依序循環 10～29 倍（每個值 15 天）。
    sqlx::query(
        "INSERT INTO daily_market_valuation (date, stock_exchange_market_id, median_pe, pe_samples, pb_samples)
         SELECT DATE '1992-01-01' + i, 0, 10 + i % 20, 3, 3 FROM generate_series(0, 299) AS i",
    )
    .execute(pool)
    .await
    .expect("種入歷史估值");
    // 03-01 為 12 倍（低於它的只有 10、11，約第 10 百分位）；03-08 為 25 倍（約第 75 百分位）。
    for (date, median_pe) in [(day(1), 12), (day(8), 25)] {
        sqlx::query(
            "INSERT INTO daily_market_valuation (date, stock_exchange_market_id, median_pe, pe_samples, pb_samples)
             VALUES ($1, 0, $2, 3, 3)",
        )
        .bind(date)
        .bind(rust_decimal::Decimal::from(median_pe))
        .execute(pool)
        .await
        .expect("種入換股日估值");
    }

    let (status, json) = query("limit=2&market_filter_pe_percentile=50").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["market_filter_pe_percentile"], 50);
    assert_eq!(json["periods"][0]["market_filtered"], false);
    assert_eq!(json["periods"][1]["market_filtered"], true);
    assert_eq!(json["summary"]["market_filtered_periods"], 1);
    // 門檻 80：25 倍只在第 75 百分位，兩期都通過。
    let (_, json) = query("limit=2&market_filter_pe_percentile=80").await;
    assert_eq!(json["summary"]["market_filtered_periods"], 0);
    // 與固定倍數濾網併用：任一不符就空手（03-01 的 12 倍高於 11 倍）。
    let (_, json) = query("limit=2&market_filter_pe_percentile=80&market_filter_max_pe=11").await;
    assert_eq!(json["summary"]["market_filtered_periods"], 2);
    // 上櫃沒有歷史（不足 250 天）：都通過。
    let (_, json) = query("market=tpex&market_filter_pe_percentile=1").await;
    assert_eq!(json["summary"]["market_filtered_periods"], 0);
    let (_, json) = query("limit=2").await;
    assert!(json["market_filter_pe_percentile"].is_null(), "{json}");
    clear().await;

    for bad in [
        "market_filter_pe_percentile=0",
        "market_filter_pe_percentile=100",
    ] {
        let (status, json) = query(&format!("limit=2&{bad}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {json}");
    }
}
