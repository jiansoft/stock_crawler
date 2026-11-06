//! 條件選股回測 endpoint 整合測試：CANSLIM 的 C（單季 EPS 年增）、A（年度 EPS 成長）、N（52 週新高）
//! 只用換股當天已公布的資料。

use super::*;

/// 種入季報、年度 EPS 與 52 週新高訊號。
///
/// 1993-03 換股時最新已公布的季報是 1992 Q3（Q4 要 4/1 起），最新年度是 1991（1992 年度要 1993-04-01 起）：
/// - 單季：BA 1991 Q3 1 → 1992 Q3 2（+100%）；BB 去年同季虧損（無法計算）；BC +10%。BA 另有尚未公布的
///   1992 Q4（對 1991 Q4 +800%），BC 尚未公布的 Q4 也高成長，提前用到就會讓 BC 通過。
/// - 年度：BA 1988～1991 為 1.0、1.3、1.7、2.2（最低 +29.41%），另有尚未公布的 1992 年 0.5（用到就變負）；
///   BB 1.0、1.1、1.2、1.3（最低 +8.33%）；BC 沒有年度資料。
/// - 52 週新高：BA 02-25、BB 03-05。
async fn seed_canslim(pool: &sqlx::PgPool) {
    sqlx::query(
        "INSERT INTO financial_statement (security_code, year, quarter, earnings_per_share) VALUES
             ('79979BA', 1991, 'Q3', 1), ('79979BA', 1992, 'Q3', 2),
             ('79979BA', 1991, 'Q4', 1), ('79979BA', 1992, 'Q4', 9),
             ('79979BB', 1991, 'Q3', -1), ('79979BB', 1992, 'Q3', 1),
             ('79979BC', 1991, 'Q3', 2), ('79979BC', 1992, 'Q3', 2.2),
             ('79979BC', 1991, 'Q4', 1), ('79979BC', 1992, 'Q4', 5),
             ('79979BA', 1988, '', 1.0), ('79979BA', 1989, '', 1.3), ('79979BA', 1990, '', 1.7),
             ('79979BA', 1991, '', 2.2), ('79979BA', 1992, '', 0.5),
             ('79979BB', 1988, '', 1.0), ('79979BB', 1989, '', 1.1), ('79979BB', 1990, '', 1.2),
             ('79979BB', 1991, '', 1.3)",
    )
    .execute(pool)
    .await
    .expect("種入季報與年度 EPS");
    sqlx::query(
        "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description) VALUES
             ('1993-02-25', '79979BA', '52_week', 'new_high', '52 週新高'),
             ('1993-03-05', '79979BB', '52_week', 'new_high', '52 週新高')",
    )
    .execute(pool)
    .await
    .expect("種入 52 週新高");
}

/// C、A、N 三個條件各自篩選，且不會提前用到尚未公布的 Q4 季報與年度 EPS；回應回填參數，超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_canslim_filters_use_published_data_only() {
    let Some(pool) = seeded("screen_backtest_canslim_filters_use_published_data_only").await else {
        return;
    };
    seed_canslim(pool).await;

    // C：單季 EPS 年增 ≥ 50% 只有 BA（03-01 報酬 10%、03-08 3%）；BC 只有 +10%，尚未公布的 Q4 不算。
    let (status, json) = query("limit=2&min_eps_yoy_percent=50").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_eps_yoy_percent"], 50.0);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(3.0)),
        ]
    );
    // 下限 -100% 也排除去年同季虧損的 BB：03-01 為 BA、BC（10%、2%），03-08 的 BC 沒有報酬。
    let (_, json) = query("limit=2&min_eps_yoy_percent=-100").await;
    assert_eq!(periods(&json)[0], ("1993-03-01".to_owned(), 2, Some(6.0)));

    // A：近 3 年每年 ≥ 25% 只有 BA；1992 年度 0.5 元要 1993-04-01 才公布，不會讓 BA 被排除。
    let (status, json) = query("limit=2&min_annual_eps_growth_percent=25").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_annual_eps_growth_percent"], 25.0);
    assert_eq!(periods(&json)[0], ("1993-03-01".to_owned(), 1, Some(10.0)));
    // 門檻 5% 時 BB（最低 +8.33%）也通過；BC 沒有年度資料不會通過。
    let (_, json) = query("limit=2&min_annual_eps_growth_percent=5").await;
    assert_eq!(periods(&json)[0], ("1993-03-01".to_owned(), 2, Some(3.0)));

    // N：3 天內創新高，03-01 沒有（BA 的 02-25 在視窗外）、03-08 為 BB；10 天時 03-01 有 BA。
    let (status, json) = query("limit=2&new_high_days=3").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["new_high_days"], 3);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 0, None),
            ("1993-03-08".to_owned(), 1, Some(8.0)),
        ]
    );
    let (_, json) = query("limit=2&new_high_days=10").await;
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(8.0)),
        ]
    );
    let (_, json) = query("limit=2").await;
    assert!(json["min_eps_yoy_percent"].is_null(), "{json}");
    assert!(json["new_high_days"].is_null(), "{json}");

    for extra in [
        "min_eps_yoy_percent=-101",
        "min_annual_eps_growth_percent=10001",
        "new_high_days=0",
        "new_high_days=366",
    ] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
    }
    cleanup(pool).await;
}
