//! 條件選股回測 endpoint 整合測試：長期高 ROE（年度 ROE 連續達標年數），只用換股日已公布的年度報告。

use super::*;

/// 3 月換股看得到的是 1991 年度報告（1992 年度要 1993-04-01 才公布）。
/// - BA：1989～1991 年度 ROE 20、18、16（≥ 15 連續 3 年），尚未公布的 1992 只有 5。
/// - BB：1989 20、1990 10、1991 25（≥ 15 只有 1 年，≥ 20 也是 1 年）。
/// - BC：1989、1991 皆 30 但缺 1990（年度不相連，1 年）。
async fn seed_roe(pool: &sqlx::PgPool) {
    sqlx::query(
        "INSERT INTO financial_statement (security_code, year, quarter, earnings_per_share, return_on_equity)
         VALUES ('79979BA', 1989, '', 1, 20), ('79979BA', 1990, '', 1, 18), ('79979BA', 1991, '', 1, 16),
                ('79979BA', 1992, '', 1, 5),
                ('79979BB', 1989, '', 1, 20), ('79979BB', 1990, '', 1, 10), ('79979BB', 1991, '', 1, 25),
                ('79979BC', 1989, '', 1, 30), ('79979BC', 1991, '', 1, 30)",
    )
    .execute(pool)
    .await
    .expect("種入年度 ROE");
}

/// 只設門檻 15% 看最新一年：三檔都通過（BA 尚未公布的 1992 不算）；門檻 20% 剩 BB、BC；連續 2 年 ≥ 15% 只留 BA；
/// 年數沒搭配門檻或超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_filters_by_roe_years() {
    let Some(pool) = seeded("screen_backtest_filters_by_roe_years").await else {
        return;
    };
    seed_roe(pool).await;

    let all = vec![
        "79979BA".to_owned(),
        "79979BB".to_owned(),
        "79979BC".to_owned(),
    ];
    let (status, json) = query("limit=3&min_annual_roe_percent=15").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_annual_roe_percent"], 15.0);
    assert!(json["min_roe_years"].is_null(), "{json}");
    assert_eq!(holdings(&json), [all.clone(), all]);

    let (status, json) = query("limit=3&min_roe_years=1&min_annual_roe_percent=20").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let high = vec!["79979BB".to_owned(), "79979BC".to_owned()];
    assert_eq!(holdings(&json), [high.clone(), high]);

    let (status, json) = query("limit=3&min_roe_years=2&min_annual_roe_percent=15").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_roe_years"], 2);
    let only_a = vec!["79979BA".to_owned()];
    assert_eq!(holdings(&json), [only_a.clone(), only_a]);

    for extra in [
        "min_roe_years=2",
        "min_roe_years=0&min_annual_roe_percent=15",
        "min_roe_years=11&min_annual_roe_percent=15",
        "min_annual_roe_percent=-1",
        "min_annual_roe_percent=101",
    ] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
    }

    cleanup(pool).await;
}
