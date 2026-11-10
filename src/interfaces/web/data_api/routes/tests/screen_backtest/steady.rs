//! 條件選股回測 endpoint 整合測試：安心存股（市值下限、連續獲利年數），只用換股日已公布的股本與年度 EPS。

use super::*;

/// 3 月換股看得到的是 1992 Q3 的股本與 1991 年度 EPS（1992 年度要 1993-04-01 才公布）。
/// - 收盤：BA、BB 100 元，BC 50 元（兩個換股日相同）。
/// - 股本（千元）：BA 2,000,000（20 萬千股，市值 200 億）；BB 500,000（50 億），尚未公布的 1992 Q4 增資到
///   10,000,000，若提前用到會變 1,000 億；BC 沒有股本，改用目前發行股數 3 億股（150 億）。
/// - 年度 EPS：BA 1989～1991 皆 > 0（3 年），尚未公布的 1992 虧損；BB 1988 賺、1989 虧、1990～1991 賺（2 年）；
///   BC 1989、1991 賺但缺 1990（1 年）。
async fn seed_steady(pool: &sqlx::PgPool) {
    sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979B_'"#)
        .execute(pool)
        .await
        .expect("清除日線");
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice")
           SELECT d::date, s, close
           FROM (VALUES ('79979BA', 100), ('79979BB', 100), ('79979BC', 50)) AS q(s, close)
           CROSS JOIN (VALUES ('1993-03-01'), ('1993-03-08')) AS t(d)"#,
    )
    .execute(pool)
    .await
    .expect("種入收盤");
    sqlx::query(
        "INSERT INTO balance_sheet (stock_symbol, fiscal_year, period_type, quarter, source, share_capital)
         VALUES ('79979BA', 1992, 'single', 'Q3', 'yahoo', 2000000),
                ('79979BB', 1992, 'single', 'Q3', 'yahoo', 500000),
                ('79979BB', 1992, 'single', 'Q4', 'yahoo', 10000000)",
    )
    .execute(pool)
    .await
    .expect("種入股本");
    sqlx::query("UPDATE stocks SET issued_share = 300000000 WHERE stock_symbol = '79979BC'")
        .execute(pool)
        .await
        .expect("設定發行股數");
    sqlx::query(
        "INSERT INTO financial_statement (security_code, year, quarter, earnings_per_share)
         VALUES ('79979BA', 1989, '', 1), ('79979BA', 1990, '', 1), ('79979BA', 1991, '', 1),
                ('79979BA', 1992, '', -5),
                ('79979BB', 1988, '', 1), ('79979BB', 1989, '', -1), ('79979BB', 1990, '', 1),
                ('79979BB', 1991, '', 1),
                ('79979BC', 1989, '', 1), ('79979BC', 1991, '', 1)",
    )
    .execute(pool)
    .await
    .expect("種入年度 EPS");
}

/// `min_market_cap=100` 留 BA、BC（BC 用目前發行股數），`=180` 只留 BA；`min_profit_years=2` 留 BA、BB，`=3` 只留
/// BA；超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_filters_by_market_cap_and_profit_years() {
    let Some(pool) = seeded("screen_backtest_filters_by_market_cap_and_profit_years").await else {
        return;
    };
    seed_steady(pool).await;

    let (status, json) = query("limit=3&min_market_cap=100").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_market_cap"], 100.0);
    let large = vec!["79979BA".to_owned(), "79979BC".to_owned()];
    assert_eq!(holdings(&json), [large.clone(), large]);

    let (status, json) = query("limit=3&min_market_cap=180").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let largest = vec!["79979BA".to_owned()];
    assert_eq!(holdings(&json), [largest.clone(), largest]);

    let (status, json) = query("limit=3&min_profit_years=2").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let profitable = vec!["79979BA".to_owned(), "79979BB".to_owned()];
    assert_eq!(holdings(&json), [profitable.clone(), profitable]);

    let (status, json) = query("limit=3&min_profit_years=3").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_profit_years"], 3);
    let steady = vec!["79979BA".to_owned()];
    assert_eq!(holdings(&json), [steady.clone(), steady]);

    for extra in [
        "min_market_cap=0",
        "min_market_cap=1000001",
        "min_profit_years=0",
        "min_profit_years=31",
    ] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
    }

    cleanup(pool).await;
}
