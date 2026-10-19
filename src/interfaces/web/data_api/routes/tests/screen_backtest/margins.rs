//! 條件選股回測 endpoint 整合測試：財報三率與負債比（Yahoo 單季財報）只用換股當天已公布的那一季。

use super::*;

/// 種入 Yahoo 單季損益表與資產負債表。
///
/// 損益表（營收 100，毛利／營業利益／稅後淨利）：BA 1992 Q3 40／20／15、去年同季 30／10／5（三率三升），
/// 1992 Q4 10／5／1（4/1 才公布，3 月看不到）；BB 1992 Q3 50／30／20、去年同季 55／25／15（毛利率下降）；
/// BC 只有 1992 Q2 20／5／2（沒有去年同季）。
///
/// 資產負債表（負債比）：BA 1992 Q3 60%、BB 1992 Q3 30%；BC 1992 Q2 80%、Q4 10%（3 月看不到）。
async fn seed_statements(pool: &sqlx::PgPool) {
    sqlx::query(
        "INSERT INTO income_statement
             (stock_symbol, fiscal_year, period_type, quarter, source, revenue, gross_profit, operating_profit, net_income)
         VALUES ('79979BA', 1992, 'single', 'Q3', 'yahoo', 100, 40, 20, 15),
                ('79979BA', 1991, 'single', 'Q3', 'yahoo', 100, 30, 10, 5),
                ('79979BA', 1992, 'single', 'Q4', 'yahoo', 100, 10, 5, 1),
                ('79979BB', 1992, 'single', 'Q3', 'yahoo', 100, 50, 30, 20),
                ('79979BB', 1991, 'single', 'Q3', 'yahoo', 100, 55, 25, 15),
                ('79979BC', 1992, 'single', 'Q2', 'yahoo', 100, 20, 5, 2)",
    )
    .execute(pool)
    .await
    .expect("種入損益表");
    sqlx::query(
        "INSERT INTO balance_sheet
             (stock_symbol, fiscal_year, period_type, quarter, source, total_assets, total_liabilities)
         VALUES ('79979BA', 1992, 'single', 'Q3', 'yahoo', 100, 60),
                ('79979BB', 1992, 'single', 'Q3', 'yahoo', 100, 30),
                ('79979BC', 1992, 'single', 'Q2', 'yahoo', 100, 80),
                ('79979BC', 1992, 'single', 'Q4', 'yahoo', 100, 10)",
    )
    .execute(pool)
    .await
    .expect("種入資產負債表");
}

/// 毛利率、營業利益率下限、三率三升與負債比上限：換股日只看得到已公布的那一季，Q4 不會提前用到；
/// 超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_margins_use_published_statements_only() {
    let Some(pool) = seeded("screen_backtest_margins_use_published_statements_only").await else {
        return;
    };
    seed_statements(pool).await;

    // 毛利率 ≥ 35%：BA（40%）與 BB（50%）；BC 20% 不通過。
    let (status, json) = query("limit=2&min_gross_margin_percent=35").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_gross_margin_percent"], 35.0);
    assert_eq!(json["margins_yoy_up"], false);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(3.0)),
            ("1993-03-08".to_owned(), 2, Some(5.5)),
        ]
    );

    // 三率三升：只有 BA；BA 尚未公布的 Q4 三率大跌也不影響 3 月。
    let (status, json) = query("limit=2&margins_yoy_up=true").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["margins_yoy_up"], true);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(3.0)),
        ]
    );

    // 營業利益率 ≥ 25%、負債比 ≤ 50%：都只有 BB；BC 的 Q4 負債比 10% 尚未公布、不會用到。
    for extra in [
        "min_operating_margin_percent=25",
        "max_debt_ratio_percent=50",
    ] {
        let (status, json) = query(&format!("limit=2&{extra}")).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(
            periods(&json),
            [
                ("1993-03-01".to_owned(), 1, Some(-4.0)),
                ("1993-03-08".to_owned(), 1, Some(8.0)),
            ],
            "{extra}"
        );
    }
    let (_, json) = query("limit=2&max_debt_ratio_percent=50").await;
    assert_eq!(json["max_debt_ratio_percent"], 50.0);

    for extra in [
        "min_gross_margin_percent=101",
        "min_operating_margin_percent=-10001",
        "max_debt_ratio_percent=-1",
    ] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
    }

    cleanup(pool).await;
}
