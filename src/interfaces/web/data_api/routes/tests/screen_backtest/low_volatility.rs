//! 條件選股回測 endpoint 整合測試：低波動（年化波動度、beta 上限與低波動排序），只用換股當天的風險指標。

use super::*;

/// 兩個換股日（03-01、03-08）的風險指標：`(代號, 年化波動度, beta)`。
const RISKS: [(&str, i32, f64); 3] = [
    ("79979BA", 20, 0.5),
    ("79979BB", 40, 1.2),
    ("79979BC", 30, 0.8),
];

async fn seed_risks(pool: &sqlx::PgPool) {
    for (symbol, volatility, beta) in RISKS {
        for date in [day(1), day(8)] {
            sqlx::query(
                "INSERT INTO daily_risk_metric (date, stock_symbol, volatility_percent, beta, samples)
                 VALUES ($1, $2, $3, $4, 240)",
            )
            .bind(date)
            .bind(symbol)
            .bind(Decimal::from(volatility))
            .bind(Decimal::try_from(beta).expect("有限數值"))
            .execute(pool)
            .await
            .expect("種入風險指標");
        }
    }
}

/// 回傳每期持股代號（排序後），比較不受相對強度名次影響。
fn holdings(json: &serde_json::Value) -> Vec<Vec<String>> {
    json["periods"]
        .as_array()
        .expect("periods")
        .iter()
        .map(|period| {
            let mut symbols: Vec<String> = period["symbols"]
                .as_array()
                .expect("symbols")
                .iter()
                .filter_map(|symbol| symbol.as_str().map(str::to_owned))
                .collect();
            symbols.sort();
            symbols
        })
        .collect()
}

/// 年化波動度 ≤ 25 只留 BA；beta ≤ 1 留 BA、BC；依低波動排序兩期都是 BA、BC、BB（與預設依相對強度不同）；
/// 超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_filters_and_ranks_by_low_volatility() {
    let Some(pool) = seeded("screen_backtest_filters_and_ranks_by_low_volatility").await else {
        return;
    };
    let _ = sqlx::query("DELETE FROM daily_risk_metric WHERE stock_symbol LIKE '79979B_'")
        .execute(pool)
        .await;
    seed_risks(pool).await;

    let (status, json) = query("limit=3&max_volatility_percent=25").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["max_volatility_percent"], 25.0);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(3.0)),
        ]
    );

    let (status, json) = query("limit=3&max_beta=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["max_beta"], 1.0);
    let both = vec!["79979BA".to_owned(), "79979BC".to_owned()];
    assert_eq!(holdings(&json), [both.clone(), both]);

    let (status, json) = query("limit=3&rank_by=low_volatility").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["rank_by"], "low_volatility");
    for period in json["periods"].as_array().expect("periods") {
        assert_eq!(
            period["symbols"],
            serde_json::json!(["79979BA", "79979BC", "79979BB"]),
            "{period}"
        );
    }

    for extra in [
        "max_volatility_percent=0.5",
        "max_volatility_percent=201",
        "max_beta=-6",
        "max_beta=5.5",
    ] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
    }

    let _ = sqlx::query("DELETE FROM daily_risk_metric WHERE stock_symbol LIKE '79979B_'")
        .execute(pool)
        .await;
    cleanup(pool).await;
}
