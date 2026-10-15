//! 條件選股回測 endpoint 的資料庫語意整合測試。

use axum::http::StatusCode;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::support::{get, pool_or_skip};

/// `(代號, 市場, 產業)`；產業 24 為 CI 種入的「半導體業」。
const STOCKS: [(&str, i32, i32); 3] = [("79979BA", 2, 24), ("79979BB", 2, 99), ("79979BC", 4, 24)];

/// 有相對強度的交易日（1993-03-dd）：持有 5 日時第 1、6 天換股，第 11 天還沒有報酬。
const DAYS: [u32; 11] = [1, 2, 3, 4, 5, 8, 9, 10, 11, 12, 15];

/// `(日, 代號, 相對強度分數, 5 日報酬)`；03-08 的 BC 沒有報酬（之後完全沒有收盤）。
const SCORES: [(u32, &str, i16, Option<i64>); 6] = [
    (1, "79979BA", 90, Some(10)),
    (1, "79979BB", 60, Some(-4)),
    (1, "79979BC", 80, Some(2)),
    (8, "79979BA", 50, Some(3)),
    (8, "79979BB", 95, Some(8)),
    (8, "79979BC", 70, None),
];

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(1993, 3, d).expect("固定日期合法")
}

/// 刪除種入的相對強度、報酬、訊號、籌碼與股票主檔。
async fn cleanup(pool: &sqlx::PgPool) {
    let symbols: Vec<&str> = STOCKS.iter().map(|row| row.0).collect();
    for sql in [
        "DELETE FROM daily_relative_strength WHERE stock_symbol = ANY($1)",
        "DELETE FROM daily_forward_return WHERE stock_symbol = ANY($1)",
        "DELETE FROM daily_technical_signal WHERE stock_symbol = ANY($1)",
        "DELETE FROM chip_daily WHERE stock_symbol = ANY($1)",
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(&symbols).execute(pool).await;
    }
    let _ = sqlx::query(
        "DELETE FROM daily_forward_return WHERE stock_symbol = '0050' AND date BETWEEN $1 AND $2",
    )
    .bind(day(1))
    .bind(day(31))
    .execute(pool)
    .await;
}

async fn seed(pool: &sqlx::PgPool) {
    for (symbol, market_id, industry) in STOCKS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, $1, $1, $2, $3)"#,
        )
        .bind(symbol)
        .bind(market_id)
        .bind(industry)
        .execute(pool)
        .await
        .expect("種入 stocks");
    }
    // 每個交易日都有三檔的分數（預設 50），換股日用 SCORES 的分數。
    for d in DAYS {
        for (symbol, _, _) in STOCKS {
            let score = SCORES
                .iter()
                .find(|row| row.0 == d && row.1 == symbol)
                .map_or(50, |row| row.2);
            sqlx::query(
                "INSERT INTO daily_relative_strength (date, stock_symbol, weighted_return, rs_score, rank, universe)
                 VALUES ($1, $2, 0, $3, 1, 3)",
            )
            .bind(day(d))
            .bind(symbol)
            .bind(score)
            .execute(pool)
            .await
            .expect("種入相對強度");
        }
    }
    for (d, symbol, _, return_5d) in SCORES {
        sqlx::query(
            "INSERT INTO daily_forward_return (date, stock_symbol, return_5d) VALUES ($1, $2, $3)",
        )
        .bind(day(d))
        .bind(symbol)
        .bind(return_5d.map(Decimal::from))
        .execute(pool)
        .await
        .expect("種入報酬");
    }
    // 0050：03-01 買進 5 日報酬 4%；03-08 沒有資料（彙總時算 0）。
    sqlx::query(
        "INSERT INTO daily_forward_return (date, stock_symbol, return_5d) VALUES ($1, '0050', 4)",
    )
    .bind(day(1))
    .execute(pool)
    .await
    .expect("種入 0050 報酬");
    // BB 在 03-03 出現 KD 黃金交叉（落在 03-08 換股的回看視窗 03-02～03-08 內）。
    sqlx::query(
        "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
         VALUES ($1, '79979BB', 'kd', 'golden_cross', 'KD 黃金交叉')",
    )
    .bind(day(3))
    .execute(pool)
    .await
    .expect("種入訊號");
    // 外資：BA 02-26、03-01 連買 2 天；BB 03-05、03-08 連買 2 天。
    sqlx::query(
        r#"INSERT INTO chip_daily ("date", stock_symbol, foreign_net, trust_net, dealer_net) VALUES
           ('1993-02-26', '79979BA', 100, 0, 0), ('1993-03-01', '79979BA', 100, 0, 0),
           ('1993-03-05', '79979BB', 100, 0, 0), ('1993-03-08', '79979BB', 100, 0, 0)"#,
    )
    .execute(pool)
    .await
    .expect("種入籌碼");
    crate::infra::database::repository::chip::refresh_streaks_for_tests().await;
}

/// 呼叫 endpoint（固定 03-01～03-15、持有 5 日），回傳狀態與 JSON。
async fn query(extra: &str) -> (StatusCode, serde_json::Value) {
    get(&format!(
        "/api/v1/stocks/screen/backtest?from=1993-03-01&to=1993-03-15&hold_days=5&{extra}"
    ))
    .await
}

/// 每期 `(日期, 檔數, 平均報酬)`。
fn periods(json: &serde_json::Value) -> Vec<(String, i64, Option<f64>)> {
    json["periods"]
        .as_array()
        .expect("periods array")
        .iter()
        .map(|period| {
            (
                period["date"].as_str().unwrap_or_default().to_owned(),
                period["holdings"].as_i64().unwrap_or_default(),
                period["avg_return"].as_f64(),
            )
        })
        .collect()
}

/// 換股日、相對強度排序取前 N 檔、沒有報酬的股票不列入平均、大盤基準、交易成本、各種條件、空手與
/// 參數驗證。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("screen_backtest_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool).await;

    // 每期取相對強度前 2 檔：03-01 BA、BC（10、2），03-08 BB、BC（BC 沒有報酬，只算 BB 的 8）。
    // 03-15 還沒有報酬，不列入。
    let (status, json) = query("limit=2").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["to"], "1993-03-15");
    assert_eq!(
        (json["hold_days"].as_u64(), json["limit"].as_u64()),
        (Some(5), Some(2))
    );
    assert_eq!(json["adjusted"], true);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(6.0)),
            ("1993-03-08".to_owned(), 2, Some(8.0)),
        ]
    );
    assert_eq!(
        json["periods"][0]["symbols"],
        serde_json::json!(["79979BA", "79979BC"])
    );
    // 大盤基準：03-01 三檔平均 8/3；03-08 只有兩檔有報酬，平均 5.5。
    assert_eq!(json["periods"][0]["benchmark_return"], 2.67);
    assert_eq!(json["periods"][1]["benchmark_return"], 5.5);
    let summary = &json["summary"];
    assert_eq!(summary["periods"], 2);
    assert_eq!(summary["invested_periods"], 2);
    assert_eq!(summary["total_return"], 14.48);
    assert_eq!(summary["benchmark_total_return"], 8.31);
    assert_eq!(summary["win_rate"], 1.0);
    assert_eq!(summary["beat_market_rate"], 1.0);
    assert_eq!(summary["max_drawdown"], 0.0);
    // 0050：03-01 報酬 4%、03-08 缺值算 0；兩期都勝過 0050。
    assert_eq!(json["etf_benchmark"], "0050");
    assert_eq!(json["periods"][0]["etf_return"], 4.0);
    assert!(json["periods"][1]["etf_return"].is_null());
    assert_eq!(summary["etf_total_return"], 4.0);
    assert_eq!(summary["etf_max_drawdown"], 0.0);
    assert_eq!(summary["beat_etf_rate"], 1.0);
    // 年度彙總：兩期都在 1993 年。
    let years = json["years"].as_array().expect("years array");
    assert_eq!(years.len(), 1);
    assert_eq!(
        (years[0]["year"].as_i64(), years[0]["periods"].as_i64()),
        (Some(1993), Some(2))
    );
    assert_eq!(years[0]["total_return"], 14.48);
    assert_eq!(years[0]["benchmark_total_return"], 8.31);
    assert_eq!(years[0]["etf_total_return"], 4.0);
    // 交易成本：03-01 全部買進（0.1425%）；03-08 賣出 BA（一半，0.4425%）、買進 BB（一半，0.1425%）。
    assert_eq!(json["fee_discount"], 1.0);
    assert_eq!(json["periods"][0]["turnover"], 1.0);
    assert_eq!(json["periods"][0]["return_after_costs"], 5.86);
    assert_eq!(json["periods"][1]["turnover"], 0.5);
    assert_eq!(json["periods"][1]["return_after_costs"], 7.71);
    assert_eq!(summary["avg_turnover"], 0.75);
    // 1.058575 × 1.077075 − 1 ≈ 14.02%。
    assert_eq!(summary["total_return_after_costs"], 14.02);
    let (_, free) = query("limit=2&fee_discount=0").await;
    assert_eq!(free["periods"][0]["return_after_costs"], 6.0);
    assert_eq!(free["periods"][1]["return_after_costs"], 7.85);

    // 上櫃只有 BC：03-08 母體沒有任何報酬，那一期不列入。
    let (_, json) = query("market=tpex").await;
    assert_eq!(periods(&json), [("1993-03-01".to_owned(), 1, Some(2.0))]);
    assert_eq!(json["periods"][0]["benchmark_return"], 2.0);

    // 產業篩選只影響選股：03-08 的 BA 3、BC 沒有報酬。
    let (_, json) = query("industry=%E5%8D%8A%E5%B0%8E%E9%AB%94%E6%A5%AD").await;
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(6.0)),
            ("1993-03-08".to_owned(), 2, Some(3.0)),
        ]
    );
    assert_eq!(json["periods"][1]["benchmark_return"], 5.5);

    // 相對強度門檻：03-01 BA、03-08 BB。
    let (_, json) = query("min_rs_score=85").await;
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(8.0)),
        ]
    );

    // 訊號條件：03-01 回看視窗內沒有 KD 黃金交叉 → 空手；03-08 視窗 03-02～03-08 有 BB。
    let (_, json) = query("indicator=kd&kind=golden_cross").await;
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 0, None),
            ("1993-03-08".to_owned(), 1, Some(8.0)),
        ]
    );
    assert_eq!(json["periods"][0]["symbols"], serde_json::json!([]));
    assert_eq!(json["summary"]["invested_periods"], 1);
    assert_eq!(json["summary"]["total_return"], 8.0);
    assert_eq!(json["summary"]["win_rate"], 0.5);
    assert_eq!(json["indicator"], "kd");

    // 籌碼條件：外資連買 2 天，03-01 BA、03-08 BB；投信沒有連買 → 全部空手。
    let (_, json) = query("min_foreign_streak=2").await;
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(8.0)),
        ]
    );
    let (_, json) = query("min_trust_streak=1").await;
    assert_eq!(json["summary"]["invested_periods"], 0);
    assert_eq!(json["summary"]["total_return"], 0.0);

    for (extra, status) in [
        ("limit=0", StatusCode::UNPROCESSABLE_ENTITY),
        ("limit=101", StatusCode::UNPROCESSABLE_ENTITY),
        ("fee_discount=1.5", StatusCode::UNPROCESSABLE_ENTITY),
        ("fee_discount=-0.1", StatusCode::UNPROCESSABLE_ENTITY),
        ("market=otc", StatusCode::UNPROCESSABLE_ENTITY),
        ("kind=breakout", StatusCode::UNPROCESSABLE_ENTITY),
        ("indicator=ma10", StatusCode::UNPROCESSABLE_ENTITY),
        ("min_rs_score=0", StatusCode::UNPROCESSABLE_ENTITY),
        (
            "min_rs_score=90&max_rs_score=80",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("min_foreign_streak=21", StatusCode::UNPROCESSABLE_ENTITY),
        (
            "industry=%E4%B8%8D%E5%AD%98%E5%9C%A8",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let (actual, json) = query(extra).await;
        assert_eq!(actual, status, "{extra}: {json}");
        assert!(json["error"].is_string(), "{extra}: {json}");
    }
    for (path, status) in [
        ("hold_days=10", StatusCode::UNPROCESSABLE_ENTITY),
        (
            "from=1993-03-16&to=1993-03-15",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "from=1983-03-01&to=1993-03-15",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("from=1993-3-01", StatusCode::UNPROCESSABLE_ENTITY),
        ("to=1900-01-01", StatusCode::NOT_FOUND),
    ] {
        let (actual, json) = get(&format!("/api/v1/stocks/screen/backtest?{path}")).await;
        assert_eq!(actual, status, "{path}: {json}");
    }

    cleanup(pool).await;
}
