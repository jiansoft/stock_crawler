//! 條件選股回測 endpoint 整合測試：Piotroski F 分數只用換股當天已公布的年度。

use super::*;

/// 種入每年度 F 分數。1993-03 換股時已公布的最新年度是 1991（1992 年度要 1993-04-01 起）：
/// - BA：1991 年 8 分，另有尚未公布的 1992 年 2 分（提前用到就會被排除）。
/// - BB：1991 年 5 分。
/// - BC：只有 1990 年 9 分，落後換股年 3 年，不算數。
async fn seed_f_scores(pool: &sqlx::PgPool) {
    sqlx::query(
        "INSERT INTO financial_f_score_history (stock_symbol, fiscal_year, f_score, f_score_evaluated) VALUES
             ('79979BA', 1991, 8, 9), ('79979BA', 1992, 2, 9),
             ('79979BB', 1991, 5, 9),
             ('79979BC', 1990, 9, 9)",
    )
    .execute(pool)
    .await
    .expect("種入 F 分數歷史");
}

/// F 分數下限只看已公布且不過期的年度；回應回填參數，超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_f_score_uses_published_years_only() {
    let Some(pool) = seeded("screen_backtest_f_score_uses_published_years_only").await else {
        return;
    };
    seed_f_scores(pool).await;

    // ≥ 7 只有 BA（03-01 報酬 10%、03-08 3%）；尚未公布的 1992 年 2 分不算，BC 的 1990 年過期。
    let (status, json) = query("limit=2&min_f_score=7").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_f_score"], 7);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(3.0)),
        ]
    );
    // ≥ 5 時 BB 也通過：03-01 為 BA、BB（10%、-4%）。
    let (_, json) = query("limit=2&min_f_score=5").await;
    assert_eq!(periods(&json)[0], ("1993-03-01".to_owned(), 2, Some(3.0)));

    let (_, json) = query("limit=2").await;
    assert!(json["min_f_score"].is_null(), "{json}");
    let (status, json) = query("min_f_score=10").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{json}");
    cleanup(pool).await;
}
