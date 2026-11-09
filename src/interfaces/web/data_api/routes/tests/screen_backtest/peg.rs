//! 條件選股回測 endpoint 整合測試：PEG（本益比 ÷ 近四季 EPS 成長率）上限與排序，只用換股當天已公布的那一季。

use super::*;

/// 種入 Yahoo 單季 EPS 與換股日官方本益比。3 月換股看得到的是 1992 Q3：近四季 1991 Q4～1992 Q3、前四季
/// 1990 Q4～1991 Q3。
/// - BA：近四季每季 2（8）、前四季每季 1（4）→ 成長 100%；本益比 20 → PEG 0.2。尚未公布的 1992 Q4 EPS −100，
///   若提前用到成長率會變負、不會通過。
/// - BB：近四季每季 3（12）、前四季每季 2（8）→ 成長 50%；本益比 30 → PEG 0.6。
/// - BC：近四季每季 5（20）、前四季每季 2（8）→ 成長 150%（超過 100% 上限）；本益比 10，若不設上限 PEG 0.07 會排
///   第一，實際沒有 PEG。
async fn seed_peg(pool: &sqlx::PgPool) {
    sqlx::query(
        "INSERT INTO income_statement (stock_symbol, fiscal_year, period_type, quarter, source, eps)
         SELECT s, y, 'single', q, 'yahoo', CASE WHEN y * 4 + SUBSTRING(q FROM 2)::int >= 1991 * 4 + 4 THEN recent ELSE prev END
         FROM (VALUES ('79979BA', 2, 1), ('79979BB', 3, 2), ('79979BC', 5, 2)) AS e(s, recent, prev)
         CROSS JOIN (VALUES (1990, 'Q4'), (1991, 'Q1'), (1991, 'Q2'), (1991, 'Q3'),
                            (1991, 'Q4'), (1992, 'Q1'), (1992, 'Q2'), (1992, 'Q3')) AS p(y, q)
         UNION ALL SELECT '79979BA', 1992, 'single', 'Q4', 'yahoo', -100",
    )
    .execute(pool)
    .await
    .expect("種入損益表");
    sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979B_'"#)
        .execute(pool)
        .await
        .expect("清除日線");
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "PriceEarningRatio")
           SELECT d::date, s, pe
           FROM (VALUES ('79979BA', 20), ('79979BB', 30), ('79979BC', 10)) AS q(s, pe)
           CROSS JOIN (VALUES ('1993-03-01'), ('1993-03-08')) AS t(d)"#,
    )
    .execute(pool)
    .await
    .expect("種入本益比");
}

/// `max_peg=0.5` 只留 BA；`max_peg=1` 留 BA、BB；依 PEG 排序兩期都是 BA、BB、BC（沒有 PEG 的排最後），與預設依
/// 相對強度（03-08 BB 在前）不同；超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_filters_and_ranks_by_peg() {
    let Some(pool) = seeded("screen_backtest_filters_and_ranks_by_peg").await else {
        return;
    };
    seed_peg(pool).await;

    let (status, json) = query("limit=3&max_peg=0.5").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["max_peg"], 0.5);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(3.0)),
        ]
    );
    let (_, json) = query("limit=3&max_peg=1").await;
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(3.0)),
            ("1993-03-08".to_owned(), 2, Some(5.5)),
        ]
    );

    let (status, json) = query("limit=3&rank_by=peg").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["rank_by"], "peg");
    assert_eq!(json["max_peg"], serde_json::Value::Null);
    for period in json["periods"].as_array().expect("periods") {
        assert_eq!(
            period["symbols"],
            serde_json::json!(["79979BA", "79979BB", "79979BC"]),
            "{period}"
        );
    }

    for extra in ["max_peg=0", "max_peg=10.5"] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
    }

    cleanup(pool).await;
}
