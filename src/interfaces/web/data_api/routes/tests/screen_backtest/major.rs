//! 條件選股回測 endpoint 整合測試：千張大戶持股比例變化只用換股日以前已公布的週。

use super::*;

/// 種入 1993 年的集保週資料（千張比例 %）。03-08 與換股日同一天（人為的陷阱週），換股日當天還看不到；
///
/// | 代號 | 02-19 | 02-26 | 03-05 | 03-08 | 03-12 |
/// |---|---|---|---|---|---|
/// | BA | 50 | 51 | 51 | 60 | 60 |
/// | BB | 50 | 49 | 48 | 70 | 70 |
/// | BC | 50 | 52 | 53 | 53 | 54 |
async fn seed_holders(pool: &sqlx::PgPool) {
    for (symbol, percents) in [
        ("79979BA", [50, 51, 51, 60, 60]),
        ("79979BB", [50, 49, 48, 70, 70]),
        ("79979BC", [50, 52, 53, 53, 54]),
    ] {
        for (d, percent) in [(19, 2), (26, 2), (5, 3), (8, 3), (12, 3)]
            .into_iter()
            .zip(percents)
            .map(|((d, m), percent)| (NaiveDate::from_ymd_opt(1993, m, d), percent))
        {
            sqlx::query(
                r#"INSERT INTO holder_distribution ("date", stock_symbol, major_holders, major_percent, total_holders)
                   VALUES ($1, $2, 10, $3, 100)"#,
            )
            .bind(d.expect("固定日期合法"))
            .bind(symbol)
            .bind(Decimal::from(percent))
            .execute(pool)
            .await
            .expect("種入集保週資料");
        }
    }
}

/// 1 週變化 ≥ 0.5：03-01 看 02-26 對 02-19（BA +1、BC +2）；03-08 看 03-05 對 02-26，只有 BC（+1），
/// BB 若偷看 03-08 的 70 會變成 +22；03-15 看 03-12 對 03-08，只有 BC。起日太早或超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_major_change_uses_published_weeks_only() {
    let Some(pool) = seeded("screen_backtest_major_change_uses_published_weeks_only").await else {
        return;
    };
    seed_holders(pool).await;

    let (status, json) = query("limit=2&min_major_change=0.5&major_change_weeks=1").await;
    let (_, plain) = query("limit=2").await;
    // 第一週 02-19 加 2 週是 03-05，起日 03-01 不夠。
    let (early_status, early) = query("limit=2&min_major_change=0.5&major_change_weeks=2").await;
    let (range_status, _) = query("limit=2&min_major_change=100.5").await;
    let (weeks_status, _) = query("limit=2&min_major_change=0&major_change_weeks=13").await;
    cleanup(pool).await;

    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_major_change"], 0.5);
    assert_eq!(json["major_change_weeks"], 1);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(6.0)),
            ("1993-03-08".to_owned(), 1, None),
        ]
    );
    let current: Vec<&str> = json["current"]["holdings"]
        .as_array()
        .expect("目前持股")
        .iter()
        .filter_map(|holding| holding["symbol"].as_str())
        .collect();
    assert_eq!(current, ["79979BC"]);

    assert!(plain["min_major_change"].is_null(), "{plain}");
    assert!(plain["major_change_weeks"].is_null(), "{plain}");

    assert_eq!(early_status, StatusCode::UNPROCESSABLE_ENTITY, "{early}");
    assert!(
        early["error"]
            .as_str()
            .is_some_and(|message| message.contains("1993-03-05")),
        "{early}"
    );
    assert_eq!(range_status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(weeks_status, StatusCode::UNPROCESSABLE_ENTITY);
}
