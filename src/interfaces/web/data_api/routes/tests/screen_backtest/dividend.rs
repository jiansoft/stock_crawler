//! 條件選股回測 endpoint 整合測試：存股條件（殖利率、填息率、平均填息天數、連續配息年數、股利成長年數）
//! 只用換股當天已經知道的除息事件。

use super::*;

/// 種入填息事件與換股日收盤（BA、BB 兩個換股日都收 40、50 元；BC 沒有收盤）：
///
/// | 代號 | 除息日（現金股利、第幾天填息、觀察期滿日） |
/// |---|---|
/// | BA | 1990-08-01、1991-08-01、1992-08-01（2 元、10 天）；1993-03-05（1 元、1 天，觀察期 08-01 才滿） |
/// | BB | 1992-09-01（1 元、未填息、1993-01-15 滿） |
/// | BC | 1991-01-10（3 元、20 天、1991-06-01 滿）：最近一次除息距換股日超過 400 天 |
async fn seed_dividends(pool: &sqlx::PgPool) {
    sqlx::query(
        "INSERT INTO dividend_fill_event (stock_symbol, ex_date, cash_dividend, days_to_fill, mature_date) VALUES
             ('79979BA', '1990-08-01', 2, 10, '1990-12-01'),
             ('79979BA', '1991-08-01', 2, 10, '1991-12-01'),
             ('79979BA', '1992-08-01', 2, 10, '1992-12-01'),
             ('79979BA', '1993-03-05', 1, 1, '1993-08-01'),
             ('79979BB', '1992-09-01', 1, NULL, '1993-01-15'),
             ('79979BC', '1991-01-10', 3, 20, '1991-06-01')",
    )
    .execute(pool)
    .await
    .expect("種入填息事件");
    for (date, symbol, close) in [
        ("1993-03-01", "79979BA", 40),
        ("1993-03-08", "79979BA", 40),
        ("1993-03-01", "79979BB", 50),
        ("1993-03-08", "79979BB", 50),
    ] {
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice") VALUES ($1::date, $2, $3)
               ON CONFLICT (stock_symbol, "Date") DO UPDATE SET "ClosingPrice" = EXCLUDED."ClosingPrice""#,
        )
        .bind(date)
        .bind(symbol)
        .bind(Decimal::from(close))
        .execute(pool)
        .await
        .expect("種入換股日收盤");
    }
}

/// 有持股的期別 `(日期, 檔數, 平均報酬)`。
fn held(json: &serde_json::Value) -> Vec<(String, i64, Option<f64>)> {
    periods(json)
        .into_iter()
        .filter(|period| period.1 > 0)
        .collect()
}

/// 殖利率看最近一次除息往回 350 天、填息只算觀察期已滿的事件、連續配息年數只數換股日以前的除息；
/// 回應回填參數，超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_dividend_filters_use_known_events_only() {
    let Some(pool) = seeded("screen_backtest_dividend_filters_use_known_events_only").await else {
        return;
    };
    seed_dividends(pool).await;

    // 殖利率：BA 03-01 為 2 ÷ 40 = 5%，03-08 加上 03-05 的 1 元為 7.5%；BB 2%；BC 已停止配息。
    let (status, json) = query("limit=2&min_dividend_yield_percent=4").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_dividend_yield_percent"], 4.0);
    assert_eq!(
        held(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(3.0)),
        ]
    );
    let (_, json) = query("limit=2&min_dividend_yield_percent=6").await;
    assert_eq!(held(&json), [("1993-03-08".to_owned(), 1, Some(3.0))]);

    // 填息率：BA、BC 100%，BB 0%；03-08 的 BC 沒有報酬。
    let (_, json) = query("limit=2&min_fill_rate_percent=50").await;
    assert_eq!(json["min_fill_rate_percent"], 50.0);
    assert_eq!(
        held(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(6.0)),
            ("1993-03-08".to_owned(), 2, Some(3.0)),
        ]
    );
    // 平均填息天數：BA 10 天、BC 20 天，BB 沒有填息不通過；03-05 那次觀察期未滿，不會把 BA 拉到 7.75 天。
    let (_, json) = query("limit=2&max_avg_fill_days=15").await;
    assert_eq!(json["max_avg_fill_days"], 15.0);
    assert_eq!(
        held(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(3.0)),
        ]
    );
    let (_, json) = query("limit=2&max_avg_fill_days=9").await;
    assert!(held(&json).is_empty(), "{json}");

    // 連續配息年數：BA 3 年（03-08 起 4 年）、BB 1 年、BC 最近一次是 1991 年不算。
    let (_, json) = query("limit=2&min_dividend_years=2").await;
    assert_eq!(json["min_dividend_years"], 2);
    assert_eq!(
        held(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(3.0)),
        ]
    );
    let (_, json) = query("limit=2&min_dividend_years=1").await;
    assert_eq!(
        held(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(3.0)),
            ("1993-03-08".to_owned(), 2, Some(5.5)),
        ]
    );

    // 股利成長年數：只看 1992 年以前的完整年度。BA 1990～1992 每年 2 元為 2 年（1993-03-05 那次不算，
    // 所以不會因為 1993 只有 1 元而歸零）、BB 只有 1992 年為 0、BC 1992 年沒配息為 0。
    let (_, json) = query("limit=2&min_dividend_growth_years=2").await;
    assert_eq!(json["min_dividend_growth_years"], 2);
    assert_eq!(
        held(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(3.0)),
        ]
    );
    let (_, json) = query("limit=2&min_dividend_growth_years=3").await;
    assert!(held(&json).is_empty(), "{json}");

    let (_, json) = query("limit=2").await;
    for field in [
        "min_dividend_yield_percent",
        "min_fill_rate_percent",
        "max_avg_fill_days",
        "min_dividend_years",
        "min_dividend_growth_years",
    ] {
        assert!(json[field].is_null(), "{field}: {json}");
    }
    for extra in [
        "min_dividend_yield_percent=101",
        "min_fill_rate_percent=-1",
        "max_avg_fill_days=0",
        "min_dividend_years=31",
        "min_dividend_growth_years=0",
        "min_dividend_growth_years=31",
    ] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
    }
    cleanup(pool).await;
}

/// 殖利率在除息後遇到配股除權時，每股現金股利依股數倍數攤薄：三檔都在 1992-09-01 除息 3 元、換股日收盤 50 元
/// （未攤薄 6%），股票股利都是 10 元（股數 × 2）。BA 在兩個換股日之間（1993-03-03）除權，03-08 才降為 3%；
/// BB 與除息同一天除權，兩期都是 3%；BC 在除息前（1992-08-01）除權，現金股利已是除權後股數配的，維持 6%。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_dividend_yield_adjusts_for_stock_dividends() {
    let Some(pool) = seeded("screen_backtest_dividend_yield_adjusts_for_stock_dividends").await
    else {
        return;
    };
    sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979B_'"#)
        .execute(pool)
        .await
        .expect("清除日線");
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice")
           SELECT d::date, s, 50
           FROM (VALUES ('79979BA'), ('79979BB'), ('79979BC')) AS q(s)
           CROSS JOIN (VALUES ('1993-03-01'), ('1993-03-08')) AS t(d)"#,
    )
    .execute(pool)
    .await
    .expect("種入收盤");
    sqlx::query(
        "INSERT INTO dividend_fill_event (stock_symbol, ex_date, cash_dividend) VALUES
             ('79979BA', '1992-09-01', 3), ('79979BB', '1992-09-01', 3), ('79979BC', '1992-09-01', 3)",
    )
    .execute(pool)
    .await
    .expect("種入除息事件");
    sqlx::query(
        r#"INSERT INTO dividend (security_code, "year", year_of_dividend, quarter, cash_dividend, stock_dividend,
                                 "sum", "ex-dividend_date1", "ex-dividend_date2")
           VALUES ('79979BA', 1992, 1991, '', 3, 10, 13, '1992-09-01', '1993-03-03'),
                  ('79979BB', 1992, 1991, '', 3, 10, 13, '1992-09-01', '1992-09-01'),
                  ('79979BC', 1992, 1991, '', 3, 10, 13, '1992-09-01', '1992-08-01')"#,
    )
    .execute(pool)
    .await
    .expect("種入配股");

    let (status, json) = query("limit=3&min_dividend_yield_percent=5").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(
        holdings(&json),
        [
            vec!["79979BA".to_owned(), "79979BC".to_owned()],
            vec!["79979BC".to_owned()],
        ]
    );
    let (_, json) = query("limit=3&min_dividend_yield_percent=2.5").await;
    let all = vec![
        "79979BA".to_owned(),
        "79979BB".to_owned(),
        "79979BC".to_owned(),
    ];
    assert_eq!(holdings(&json), [all.clone(), all]);

    cleanup(pool).await;
}
