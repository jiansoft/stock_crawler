//! 條件選股回測 endpoint 整合測試：肯尼斯費雪（PSR 上限、近 3 年平均稅後淨利率），只用換股日已公布的月營收與
//! 年度損益表。

use super::*;

/// 3 月換股看得到的是 1993-01 的月營收（02 月要 03-11 起才用）與 1991 年度損益表（1992 年度要 1993-04-01 才公布）。
/// - 收盤：BA、BB 100 元，BC 50 元；發行股數皆 1 億股（市值 1,000 萬、1,000 萬、500 萬千元）。
/// - 月營收（千元）：BA 1992-02～1993-01 每月 500,000（近 12 個月 600 萬，PSR 1.67）；BB 每月 100,000（PSR 8.33），
///   尚未公布的 1993-02 有 1 億，提前用到 PSR 會掉到 0.1；BC 缺 1992-06（月份不相連，沒有 PSR）。
/// - 年度稅後淨利率：BA 1989～1991 為 10%、12%、14%（平均 12%），尚未公布的 1992 為 -50%；BB 5%、5%、20%（平均
///   10%）；BC 1989、1991 缺 1990（年度不相連）且 1991 營收為 0，沒有資料（也不會除以零出錯）。
async fn seed_psr(pool: &sqlx::PgPool) {
    sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979B_'"#)
        .execute(pool)
        .await
        .expect("清除日線");
    for sql in [
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice")
           SELECT d::date, s, close
           FROM (VALUES ('79979BA', 100), ('79979BB', 100), ('79979BC', 50)) AS q(s, close)
           CROSS JOIN (VALUES ('1993-03-01'), ('1993-03-08')) AS t(d)"#,
        "UPDATE stocks SET issued_share = 100000000 WHERE stock_symbol LIKE '79979B_'",
        r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly")
           SELECT s, s, to_char(month, 'YYYYMM')::bigint, monthly
           FROM (VALUES ('79979BA', 500000), ('79979BB', 100000), ('79979BC', 100000)) AS v(s, monthly)
           CROSS JOIN generate_series('1992-02-01'::date, '1993-01-01'::date, INTERVAL '1 month') AS month
           WHERE NOT (s = '79979BC' AND month = '1992-06-01')"#,
        r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly")
           VALUES ('79979BB', '79979BB', 199302, 100000000)"#,
        "INSERT INTO income_statement (stock_symbol, fiscal_year, period_type, quarter, source, revenue, net_income)
         VALUES ('79979BA', 1989, 'annual', 'A', 'yahoo', 1000, 100),
                ('79979BA', 1990, 'annual', 'A', 'yahoo', 1000, 120),
                ('79979BA', 1991, 'annual', 'A', 'yahoo', 1000, 140),
                ('79979BA', 1992, 'annual', 'A', 'yahoo', 1000, -500),
                ('79979BB', 1989, 'annual', 'A', 'yahoo', 1000, 50),
                ('79979BB', 1990, 'annual', 'A', 'yahoo', 1000, 50),
                ('79979BB', 1991, 'annual', 'A', 'yahoo', 1000, 200),
                ('79979BC', 1989, 'annual', 'A', 'yahoo', 1000, 300),
                ('79979BC', 1991, 'annual', 'A', 'yahoo', 0, 300)",
    ] {
        sqlx::query(sql).execute(pool).await.expect("種入費雪資料");
    }
}

/// `max_psr=2` 只留 BA、`=10` 留 BA、BB（BB 尚未公布的 2 月營收不算、BC 月份不相連）；`min_avg_net_margin_3y=11`
/// 只留 BA、`=8` 留 BA、BB（BA 尚未公布的 1992 不算、BC 年度不相連）；兩者合用；超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_filters_by_psr_and_net_margin() {
    let Some(pool) = seeded("screen_backtest_filters_by_psr_and_net_margin").await else {
        return;
    };
    seed_psr(pool).await;

    let only_a = vec!["79979BA".to_owned()];
    let a_and_b = vec!["79979BA".to_owned(), "79979BB".to_owned()];

    let (status, json) = query("limit=3&max_psr=2").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["max_psr"], 2.0);
    assert!(json["min_avg_net_margin_3y"].is_null(), "{json}");
    assert_eq!(holdings(&json), [only_a.clone(), only_a.clone()]);

    let (status, json) = query("limit=3&max_psr=10").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(holdings(&json), [a_and_b.clone(), a_and_b.clone()]);

    let (status, json) = query("limit=3&min_avg_net_margin_3y=11").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_avg_net_margin_3y"], 11.0);
    assert_eq!(holdings(&json), [only_a.clone(), only_a.clone()]);

    let (status, json) = query("limit=3&min_avg_net_margin_3y=8").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(holdings(&json), [a_and_b.clone(), a_and_b]);

    let (status, json) = query("limit=3&max_psr=5&min_avg_net_margin_3y=8").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(holdings(&json), [only_a.clone(), only_a]);

    for extra in [
        "max_psr=-1",
        "max_psr=101",
        "min_avg_net_margin_3y=-101",
        "min_avg_net_margin_3y=101",
    ] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
    }

    cleanup(pool).await;
}
