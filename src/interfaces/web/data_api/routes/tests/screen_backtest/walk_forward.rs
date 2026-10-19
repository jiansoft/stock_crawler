//! 條件選股回測 endpoint 整合測試：滾動前推最佳化。

use super::*;

/// 滾動前推：另種入 1994-03 的相對強度與 03-05 換股日的報酬，1993 為訓練期（兩期，有夏普值）、1994
/// 為樣本外；各偏移都跑一次，回傳逐年挑出的參數與彙總。訓練年數超出範圍或期間太短回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_walk_forward_rolls_yearly() {
    let Some(pool) = seeded("screen_backtest_walk_forward_rolls_yearly").await else {
        return;
    };
    let next_year = |d: u32| NaiveDate::from_ymd_opt(1994, 3, d).expect("固定日期合法");
    for d in DAYS {
        for (index, (symbol, _, _)) in STOCKS.iter().enumerate() {
            sqlx::query(
                "INSERT INTO daily_relative_strength (date, stock_symbol, weighted_return, rs_score, rank, universe)
                 VALUES ($1, $2, 0, $3, 1, 3)",
            )
            .bind(next_year(d))
            .bind(symbol)
            .bind(90 - index as i16 * 10)
            .execute(pool)
            .await
            .expect("種入 1994 相對強度");
        }
    }
    // 偏移 0 的換股日：1993-03-01、03-08、03-15（沒有報酬）、1994-03-05、03-12（未滿）。
    for (symbol, ret) in [("79979BA", 5), ("79979BB", 1), ("79979BC", -2)] {
        sqlx::query(
            "INSERT INTO daily_forward_return (date, stock_symbol, return_5d) VALUES ($1, $2, $3)",
        )
        .bind(next_year(5))
        .bind(symbol)
        .bind(Decimal::from(ret))
        .execute(pool)
        .await
        .expect("種入 1994 報酬");
    }

    let (status, json) = get(
        "/api/v1/stocks/screen/backtest/walk-forward?from=1993-03-01&to=1994-03-15&hold_days=5&limit=2&train_years=1",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["train_years"], 1);
    // 緩衝 2、3、4 × 上限 3 種 × 0050 比例 4 種。
    assert_eq!(json["candidates"], 36);
    assert_eq!(json["rebalance_offset"], 0);
    let years = json["years"].as_array().expect("years array");
    assert_eq!(years.len(), 1, "{json}");
    assert_eq!(years[0]["year"], 1994);
    assert_eq!(years[0]["periods"], 1);
    assert!(years[0]["train_sharpe_ratio"].is_number());
    assert_eq!(json["summary"]["periods"], 1);
    assert_eq!(json["baseline"]["periods"], 1);
    // 不調參數：等權持有前兩名 BA、BB（5%、1%）。
    assert_eq!(
        years[0]["baseline_return_after_costs"],
        json["baseline"]["total_return_after_costs"]
    );
    assert_eq!(json["baseline"]["total_return"], 3.0);
    assert!(json["offsets"]["annualized_return_after_costs"].is_object());
    // 各偏移樣本外都只有一期：沒有夏普值。
    assert!(json["baseline_offsets"]["sharpe_ratio"].is_null());

    for (extra, message) in [
        ("train_years=0", "train_years 必須介於 1 至 5"),
        ("train_years=6", "train_years 必須介於 1 至 5"),
    ] {
        let (status, json) = get(&format!(
            "/api/v1/stocks/screen/backtest/walk-forward?from=1993-03-01&to=1994-03-15&hold_days=5&{extra}"
        ))
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
        assert_eq!(json["error"], message);
    }
    let (status, json) = get(
        "/api/v1/stocks/screen/backtest/walk-forward?from=1993-03-01&to=1993-03-15&hold_days=5&train_years=1",
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{json}");
    let (status, _) = get(
        "/api/v1/stocks/screen/backtest/walk-forward?from=1993-03-01&to=1994-03-15&hold_days=5&train_years=1&rebalance_offset=5",
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    cleanup(pool).await;
}
