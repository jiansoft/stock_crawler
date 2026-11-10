//! 條件選股回測 endpoint 整合測試：研發成長股（近四季研發費用率下限與依研發費用率排序），只用換股日已公布的季報。

use super::*;

/// 換股日（1993-03-01、03-08）看得到的是 1991 Q4～1992 Q3（1992 Q4 要 1993-04-01 才公布）：
/// BA 每季營收 100、研發 10（10%），尚未公布的 1992 Q4 研發 0；BB 每季研發 2（2%）；BC 缺 1992 Q1（季別不相連）。
async fn seed_rd(pool: &sqlx::PgPool) {
    sqlx::query(
        "INSERT INTO income_statement (stock_symbol, fiscal_year, period_type, quarter, source, revenue, rd_expenses)
         SELECT s, y, 'single', q, 'yahoo', 100, rd
         FROM (VALUES ('79979BA', 1991, 'Q4', 10), ('79979BA', 1992, 'Q1', 10), ('79979BA', 1992, 'Q2', 10),
                      ('79979BA', 1992, 'Q3', 10), ('79979BA', 1992, 'Q4', 0),
                      ('79979BB', 1991, 'Q4', 2), ('79979BB', 1992, 'Q1', 2), ('79979BB', 1992, 'Q2', 2),
                      ('79979BB', 1992, 'Q3', 2),
                      ('79979BC', 1991, 'Q3', 9), ('79979BC', 1991, 'Q4', 9), ('79979BC', 1992, 'Q2', 9),
                      ('79979BC', 1992, 'Q3', 9)) AS t(s, y, q, rd)",
    )
    .execute(pool)
    .await
    .expect("種入損益表");
}

/// `min_rd_ratio_percent=5` 只留 BA（未公布的 Q4 不算），`=1` 留 BA、BB（BC 季別不相連）；依研發費用率排序兩期都是
/// BA、BB、BC（沒有資料排最後，與預設依相對強度不同）；超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_filters_and_ranks_by_rd_ratio() {
    let Some(pool) = seeded("screen_backtest_filters_and_ranks_by_rd_ratio").await else {
        return;
    };
    seed_rd(pool).await;

    let (status, json) = query("limit=3&min_rd_ratio_percent=5").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_rd_ratio_percent"], 5.0);
    let heavy = vec!["79979BA".to_owned()];
    assert_eq!(holdings(&json), [heavy.clone(), heavy]);

    let (status, json) = query("limit=3&min_rd_ratio_percent=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let both = vec!["79979BA".to_owned(), "79979BB".to_owned()];
    assert_eq!(holdings(&json), [both.clone(), both]);

    let (status, json) = query("limit=3&rank_by=rd_ratio").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["rank_by"], "rd_ratio");
    for period in json["periods"].as_array().expect("periods") {
        assert_eq!(
            period["symbols"],
            serde_json::json!(["79979BA", "79979BB", "79979BC"]),
            "{period}"
        );
    }

    for extra in ["min_rd_ratio_percent=-1", "min_rd_ratio_percent=101"] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
    }

    cleanup(pool).await;
}
