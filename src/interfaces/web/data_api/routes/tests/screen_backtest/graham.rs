//! 條件選股回測 endpoint 整合測試：葛拉漢（本益比 × 股價淨值比上限、流動比率下限與依乘積排序），流動比率只用
//! 換股日已公布的季報。

use super::*;

/// 換股日的本益比 × 股價淨值比：03-01 BA 10 × 1.5 = 15、BB 虧損（0）、BC 30 × 3 = 90；03-08 BA 12 × 1.2 = 14.4、
/// BB 8 × 0.8 = 6.4、BC 9 × 5 = 45。
///
/// 換股日看得到的資產負債表是 1992 Q3（Q4 要 1993-04-01 才公布）：BA 流動比率 3（尚未公布的 Q4 為 1）、BB 1.5；
/// BC 最新是 1992 Q1（落後超過 3 季，過期）。
async fn seed_graham(pool: &sqlx::PgPool) {
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
    sqlx::query(
        "INSERT INTO balance_sheet
             (stock_symbol, fiscal_year, period_type, quarter, source, current_assets, current_liabilities)
         SELECT s, y, 'single', q, 'yahoo', ca, 100
         FROM (VALUES ('79979BA', 1992, 'Q3', 300), ('79979BA', 1992, 'Q4', 100),
                      ('79979BB', 1992, 'Q3', 150),
                      ('79979BC', 1992, 'Q1', 500)) AS t(s, y, q, ca)",
    )
    .execute(pool)
    .await
    .expect("種入資產負債表");
}

/// 每期依名次排列的持股代號。
fn ranked(json: &serde_json::Value) -> Vec<serde_json::Value> {
    json["periods"]
        .as_array()
        .expect("periods")
        .iter()
        .map(|period| period["symbols"].clone())
        .collect()
}

/// 乘積 ≤ 22.5 時 03-01 只有 BA（BB 虧損）、03-08 為 BA、BB；流動比率 ≥ 2 只留 BA（未公布的 Q4 不算），≥ 1.2 留
/// BA、BB（BC 過期）；依乘積排序由低到高、虧損排最後；超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_filters_by_graham_criteria() {
    let Some(pool) = seeded("screen_backtest_filters_by_graham_criteria").await else {
        return;
    };
    seed_graham(pool).await;

    let (status, json) = query("limit=3&max_pe_pb_product=22.5").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["max_pe_pb_product"], 22.5);
    assert!(json["min_current_ratio"].is_null(), "{json}");
    let only_a = vec!["79979BA".to_owned()];
    let a_and_b = vec!["79979BA".to_owned(), "79979BB".to_owned()];
    assert_eq!(holdings(&json), [only_a.clone(), a_and_b.clone()]);

    let (status, json) = query("limit=3&min_current_ratio=2").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_current_ratio"], 2.0);
    assert_eq!(holdings(&json), [only_a.clone(), only_a.clone()]);

    let (status, json) = query("limit=3&min_current_ratio=1.2").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(holdings(&json), [a_and_b.clone(), a_and_b]);

    let (status, json) = query("limit=3&max_pe_pb_product=22.5&min_current_ratio=2").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(holdings(&json), [only_a.clone(), only_a]);

    let (status, json) = query("limit=3&rank_by=pe_pb_product").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["rank_by"], "pe_pb_product");
    assert_eq!(
        ranked(&json),
        [
            serde_json::json!(["79979BA", "79979BC", "79979BB"]),
            serde_json::json!(["79979BB", "79979BA", "79979BC"]),
        ]
    );

    for extra in [
        "max_pe_pb_product=-1",
        "max_pe_pb_product=1001",
        "min_current_ratio=-1",
        "min_current_ratio=101",
    ] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
    }

    cleanup(pool).await;
}
