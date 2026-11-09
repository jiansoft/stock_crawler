//! 條件選股回測 endpoint 整合測試：依神奇公式（Greenblatt）排序，只用換股當天已公布的那一季。

use super::*;

/// 種入 Yahoo 單季營業利益、資產負債表與換股日收盤（股本皆 100 → 股數 10）。
///
/// 3 月換股看得到的是 1992 Q3（往回四季 1991 Q4～1992 Q3）：
/// - BB：營業利益每季 25（合計 100）；流動資產 100、現金 20、流動負債 50、短期借款 10、長期負債 30、
///   不動產廠房設備 160 → 資本 (80 − 40) + 160 = 200、淨負債 20；收盤 18 → 企業價值 200。盈餘殖利率、
///   資本報酬率都是 50%。
/// - BC：每季 10（40）；流動資產 100、流動負債 50、設備 150 → 資本 200；收盤 20 → 兩者都是 20%。
/// - BA：每季 5（20）、資產負債表同 BC → 兩者都是 10%。尚未公布的 1992 Q4 營業利益 1000，若提前用到會排第一。
async fn seed_magic(pool: &sqlx::PgPool) {
    sqlx::query(
        "INSERT INTO income_statement (stock_symbol, fiscal_year, period_type, quarter, source, operating_profit)
         SELECT s, y, 'single', q, 'yahoo', v
         FROM (VALUES ('79979BB', 25), ('79979BC', 10), ('79979BA', 5)) AS e(s, v)
         CROSS JOIN (VALUES (1991, 'Q4'), (1992, 'Q1'), (1992, 'Q2'), (1992, 'Q3')) AS p(y, q)
         UNION ALL SELECT '79979BA', 1992, 'single', 'Q4', 'yahoo', 1000",
    )
    .execute(pool)
    .await
    .expect("種入損益表");
    sqlx::query(
        "INSERT INTO balance_sheet
             (stock_symbol, fiscal_year, period_type, quarter, source, current_assets, cash_and_equivalents,
              current_liabilities, short_term_debt, long_term_liabilities, property_plant_equipment, share_capital)
         VALUES ('79979BB', 1992, 'single', 'Q3', 'yahoo', 100, 20, 50, 10, 30, 160, 100),
                ('79979BC', 1992, 'single', 'Q3', 'yahoo', 100, 0, 50, 0, 0, 150, 100),
                ('79979BA', 1992, 'single', 'Q3', 'yahoo', 100, 0, 50, 0, 0, 150, 100),
                ('79979BA', 1992, 'single', 'Q4', 'yahoo', 100, 0, 50, 0, 0, 150, 100)",
    )
    .execute(pool)
    .await
    .expect("種入資產負債表");
    sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979B_'"#)
        .execute(pool)
        .await
        .expect("清除日線");
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice")
           SELECT d::date, s, c
           FROM (VALUES ('79979BB', 18), ('79979BC', 20), ('79979BA', 20)) AS q(s, c)
           CROSS JOIN (VALUES ('1993-03-01'), ('1993-03-08')) AS t(d)"#,
    )
    .execute(pool)
    .await
    .expect("種入收盤");
}

/// 依神奇公式排序：兩期都是 BB（名次 1 + 1）、BC（2 + 2）、BA（3 + 3），與預設依相對強度（03-01 BA 在前）
/// 不同；BA 尚未公布的 1992 Q4 不會提前用到。BB 目前發行股數 10 萬股（最新一季股本 100 千元 × 100 的 10 倍，
/// 面額 1 元）時股數變 100、企業價值 1,820、盈餘殖利率約 5.5% 排最後：BC（1 + 2）、BB（3 + 1）、BA（2 + 3）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_ranks_by_magic_formula_with_published_quarter() {
    let Some(pool) = seeded("screen_backtest_ranks_by_magic_formula_with_published_quarter").await
    else {
        return;
    };
    seed_magic(pool).await;

    let (status, json) = query("limit=3&rank_by=magic_formula").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["rank_by"], "magic_formula");
    for period in json["periods"].as_array().expect("periods") {
        assert_eq!(
            period["symbols"],
            serde_json::json!(["79979BB", "79979BC", "79979BA"]),
            "{period}"
        );
    }

    sqlx::query("UPDATE stocks SET issued_share = 100000 WHERE stock_symbol = '79979BB'")
        .execute(pool)
        .await
        .expect("設定發行股數");
    let (status, json) = query("limit=3&rank_by=magic_formula").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    for period in json["periods"].as_array().expect("periods") {
        assert_eq!(
            period["symbols"],
            serde_json::json!(["79979BC", "79979BB", "79979BA"]),
            "{period}"
        );
    }

    let (status, json) = query("rank_by=magic").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{json}");

    cleanup(pool).await;
}
