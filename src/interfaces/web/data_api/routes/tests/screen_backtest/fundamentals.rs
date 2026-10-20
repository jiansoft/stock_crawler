//! 條件選股回測 endpoint 整合測試：基本面條件（月營收、季報）只用換股當天已公布的資料。

use super::*;

/// 種入月營收與季報。
///
/// 月營收（年增率 %；BB 每月營收 110、去年同月 100）：BA 1992-12 +20、1993-01 −5、1993-02 +50；
/// BB 1992-11 +5、1992-12 +10、1993-01 +30；BC 只有 1992-10 +40。2 月營收要 3/11 起才算公布，所以
/// 03-01、03-08 換股時最新的是 1 月；BC 最新一筆距 1 月超過 2 個月，視為沒有資料。
///
/// 季報（EPS 元／ROE %）：BA 1992 Q3 2／5、Q4 9／20；BB 1992 Q3 −1／−3；BC 1992 Q2 3／6。Q4 要隔年
/// 4/1 起才用，3 月換股時 BA 只看得到 Q3。
async fn seed_fundamentals(pool: &sqlx::PgPool) {
    sqlx::query(
        r#"INSERT INTO "Revenue"
               ("SecurityCode", stock_symbol, "Date", "ComparedWithLastYearSameMonth", "Monthly", "LastYearThisMonth")
           VALUES
               ('79979BA', '79979BA', 199212, 20, 90, 100), ('79979BA', '79979BA', 199301, -5, 90, 100),
               ('79979BA', '79979BA', 199302, 50, 90, 100),
               ('79979BB', '79979BB', 199211, 5, 110, 100), ('79979BB', '79979BB', 199212, 10, 110, 100),
               ('79979BB', '79979BB', 199301, 30, 110, 100),
               ('79979BC', '79979BC', 199210, 40, 110, 100)"#,
    )
    .execute(pool)
    .await
    .expect("種入月營收");
    sqlx::query(
        "INSERT INTO financial_statement (security_code, year, quarter, earnings_per_share, return_on_equity)
         VALUES ('79979BA', 1992, 'Q3', 2, 5), ('79979BA', 1992, 'Q4', 9, 20),
                ('79979BB', 1992, 'Q3', -1, -3), ('79979BC', 1992, 'Q2', 3, 6)",
    )
    .execute(pool)
    .await
    .expect("種入季報");
}

/// 目前持股（03-15 換股）的代號。
fn current_symbols(json: &serde_json::Value) -> Vec<String> {
    json["current"]["holdings"]
        .as_array()
        .map(|holdings| {
            holdings
                .iter()
                .filter_map(|holding| holding["symbol"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// 月營收年增率、連續年增月數、近 3 月年增率與季報 EPS／ROE 門檻：換股日只看得到已公布的那一期，
/// 尚未公布的 2 月營收與 Q4 財報不會提前用到；超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_fundamentals_use_published_data_only() {
    let Some(pool) = seeded("screen_backtest_fundamentals_use_published_data_only").await else {
        return;
    };
    seed_fundamentals(pool).await;

    // 月營收年增率 ≥ 0：03-01、03-08 只有 BB（BA 1 月 −5%、BC 過期）；03-15 起 BA 的 2 月 +50% 已公布。
    let (status, json) = query("limit=2&min_revenue_yoy_percent=0").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_revenue_yoy_percent"], 0.0);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(-4.0)),
            ("1993-03-08".to_owned(), 1, Some(8.0)),
        ]
    );
    assert_eq!(current_symbols(&json), ["79979BA", "79979BB"]);

    // 連續年增 ≥ 3 個月：只有 BB（1992-11～1993-01）；BA 到 03-15 也只有 2 月 1 個月。
    let (status, json) = query("limit=2&min_revenue_yoy_streak=3").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(periods(&json)[0], ("1993-03-01".to_owned(), 1, Some(-4.0)));
    assert_eq!(current_symbols(&json), ["79979BB"]);

    // 近 3 月年增率 ≥ 5%：BB 為 +10%；BA 在 03-08 前只有 2 個月，03-15 起 3 個月合計 −10%。
    let (status, json) = query("limit=2&min_revenue_yoy_3m_percent=5").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(periods(&json)[1], ("1993-03-08".to_owned(), 1, Some(8.0)));
    assert_eq!(current_symbols(&json), ["79979BB"]);

    // EPS ≥ 1：BA（Q3 2 元）與 BC（Q2 3 元）；03-08 的 BC 沒有報酬，占名額但不列入平均。
    let (status, json) = query("limit=2&min_eps=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_eps"], 1.0);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(6.0)),
            ("1993-03-08".to_owned(), 2, Some(3.0)),
        ]
    );
    // EPS ≥ 5、ROE ≥ 10：只有尚未公布的 BA Q4（9 元、20%）達標，所以都沒有持股。
    for extra in ["min_eps=5", "min_roe_percent=10"] {
        let (status, json) = query(&format!("limit=2&{extra}")).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(
            periods(&json),
            [
                ("1993-03-01".to_owned(), 0, None),
                ("1993-03-08".to_owned(), 0, None),
            ],
            "{extra}"
        );
    }
    // ROE ≥ 5：BA（Q3 5%，等於門檻算通過）與 BC（6%）。
    let (_, json) = query("limit=2&min_roe_percent=5").await;
    assert_eq!(periods(&json)[0], ("1993-03-01".to_owned(), 2, Some(6.0)));

    for extra in [
        "min_revenue_yoy_percent=-101",
        "min_revenue_yoy_streak=13",
        "min_revenue_yoy_3m_percent=10001",
        "min_eps=10001",
        "min_roe_percent=-10001",
    ] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
    }

    // 依最新月營收年增率排序：1 月 BB（+30）在 BA（−5）之前，尚未公布的 2 月（BA +50）不會用到；
    // BC 沒有新鮮營收排最後。
    let (status, json) = query("limit=3&rank_by=revenue_yoy").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["rank_by"], "revenue_yoy");
    for period in json["periods"].as_array().expect("periods") {
        assert_eq!(
            period["symbols"],
            serde_json::json!(["79979BB", "79979BA", "79979BC"]),
            "{period}"
        );
    }

    cleanup(pool).await;
}

/// 營收創歷史新高：補上 1991-11～1992-10 每月 100（BB）與 1992-01～1992-11 每月 50（BA），BB 的 1 月營收改為
/// 150（高於之前 14 個月），BA 的 1 月 90 只追平 1992-12；尚未公布的 2 月（BA 改成 500）不會用到。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_revenue_record_uses_published_months_only() {
    let Some(pool) = seeded("screen_backtest_revenue_record_uses_published_months_only").await
    else {
        return;
    };
    seed_fundamentals(pool).await;
    for sql in [
        r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly")
           SELECT '79979BB', '79979BB', to_char(month, 'YYYYMM')::bigint, 100
           FROM generate_series('1991-11-01'::date, '1992-10-01'::date, INTERVAL '1 month') AS month"#,
        r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly")
           SELECT '79979BA', '79979BA', to_char(month, 'YYYYMM')::bigint, 50
           FROM generate_series('1992-01-01'::date, '1992-11-01'::date, INTERVAL '1 month') AS month"#,
        r#"UPDATE "Revenue" SET "Monthly" = 150 WHERE "SecurityCode" = '79979BB' AND "Date" = 199301"#,
        r#"UPDATE "Revenue" SET "Monthly" = 500 WHERE "SecurityCode" = '79979BA' AND "Date" = 199302"#,
    ] {
        sqlx::query(sql).execute(pool).await.expect("種入營收歷史");
    }

    let (status, json) = query("limit=3&revenue_record=true").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["revenue_record"], true);
    for period in json["periods"].as_array().expect("periods") {
        assert_eq!(
            period["symbols"],
            serde_json::json!(["79979BB"]),
            "{period}"
        );
    }
    let (_, json) = query("limit=3").await;
    assert_eq!(json["revenue_record"], false);

    cleanup(pool).await;
}
