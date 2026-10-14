//! 訊號績效與個股訊號歷史 endpoint 的資料庫語意整合測試。

use axum::http::StatusCode;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::support::{get, pool_or_skip};

/// `(代號, 市場, 產業)`；產業 24 為 CI 種入的「半導體業」。
const STOCKS: [(&str, i32, i32); 3] = [("79979PA", 2, 24), ("79979PB", 2, 99), ("79979PC", 4, 24)];

/// `(日, 代號, 來源, 種類)`，日期為 1991-10-dd。
const SIGNALS: [(u32, &str, &str, &str); 5] = [
    (1, "79979PA", "kd", "golden_cross"),
    (1, "79979PA", "52_week", "new_high"),
    (1, "79979PB", "kd", "golden_cross"),
    (1, "79979PC", "kd", "golden_cross"),
    (2, "79979PA", "kd", "golden_cross"),
];

/// `(日, 代號, 5 日, 20 日)` 報酬（%）；60、120 日一律未滿，240 日只有 10-01 的 PA（30%）。
const RETURNS: [(u32, &str, i64, Option<i64>); 6] = [
    (1, "79979PA", 4, Some(10)),
    (1, "79979PB", -2, None),
    (1, "79979PC", 6, None),
    (2, "79979PA", -1, None),
    (2, "79979PB", 1, None),
    (2, "79979PC", 3, None),
];

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(1991, 10, d).expect("固定日期合法")
}

/// 刪除種入的訊號、報酬、日報價與股票主檔。
async fn cleanup(pool: &sqlx::PgPool) {
    let symbols: Vec<&str> = STOCKS.iter().map(|row| row.0).collect();
    for sql in [
        "DELETE FROM daily_technical_signal WHERE stock_symbol = ANY($1)",
        "DELETE FROM daily_forward_return WHERE stock_symbol = ANY($1)",
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(&symbols).execute(pool).await;
    }
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
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "ChangeRange")
           VALUES ($1, '79979PA', 50, 1.5)"#,
    )
    .bind(day(1))
    .execute(pool)
    .await
    .expect("種入日報價");
    for (d, symbol, indicator, kind) in SIGNALS {
        sqlx::query(
            "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
             VALUES ($1, $2, $3, $4, $2 || ' ' || $4)",
        )
        .bind(day(d))
        .bind(symbol)
        .bind(indicator)
        .bind(kind)
        .execute(pool)
        .await
        .expect("種入訊號");
    }
    for (d, symbol, r5, r20) in RETURNS {
        sqlx::query(
            "INSERT INTO daily_forward_return (date, stock_symbol, return_5d, return_20d)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(day(d))
        .bind(symbol)
        .bind(Decimal::from(r5))
        .bind(r20.map(Decimal::from))
        .execute(pool)
        .await
        .expect("種入報酬");
    }
    sqlx::query(
        "UPDATE daily_forward_return SET return_240d = 30 WHERE stock_symbol = '79979PA' AND date = $1",
    )
    .bind(day(1))
    .execute(pool)
    .await
    .expect("種入 240 日報酬");
}

/// 找出某來源與種類的績效項目。
fn item<'a>(json: &'a serde_json::Value, indicator: &str, kind: &str) -> &'a serde_json::Value {
    json["items"]
        .as_array()
        .expect("items array")
        .iter()
        .find(|item| item["indicator"] == indicator && item["kind"] == kind)
        .unwrap_or_else(|| panic!("找不到 {indicator} {kind}：{json}"))
}

/// 平均、中位數、勝率、超額報酬與大盤基準；市場與產業篩選；參數驗證與查無資料。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn signal_performance_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("signal_performance_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool).await;

    // 迄日 10-03 沒有訊號，回推到 10-02。
    let (status, json) =
        get("/api/v1/market/signals/performance?from=1991-09-25&to=1991-10-03").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["from"], "1991-09-25");
    assert_eq!(json["to"], "1991-10-02");
    assert_eq!(json["market"], "all");
    assert_eq!(json["adjusted"], true);
    assert_eq!(json["items"][0]["indicator"], "kd", "kd 排在 52_week 之前");

    // kd 黃金交叉 4 次：5 日報酬 4、-2、6、-1；當天大盤平均 10-01 為 8/3、10-02 為 1。
    let kd = item(&json, "kd", "golden_cross");
    assert_eq!(kd["occurrences"], 4);
    let five = &kd["horizons"][0];
    assert_eq!(five["days"], 5);
    assert_eq!(five["samples"], 4);
    assert_eq!(five["avg_return"], 1.75);
    assert_eq!(five["median_return"], 1.5);
    assert_eq!(five["win_rate"], 0.5);
    assert_eq!(five["avg_excess_return"], -0.5);
    assert_eq!(five["beat_market_rate"], 0.5);
    // 20 日只有 PA 一筆 10%，與當天大盤（也只有它）相同，不算勝過大盤。
    let twenty = &kd["horizons"][1];
    assert_eq!(twenty["samples"], 1);
    assert_eq!(twenty["avg_return"], 10.0);
    assert_eq!(twenty["win_rate"], 1.0);
    assert_eq!(twenty["avg_excess_return"], 0.0);
    assert_eq!(twenty["beat_market_rate"], 0.0);
    let sixty = &kd["horizons"][2];
    assert_eq!(sixty["samples"], 0);
    assert!(sixty["avg_return"].is_null() && sixty["median_return"].is_null());
    // 240 日只有 PA 一筆 30%，當天大盤也只有它。
    let long = &kd["horizons"][4];
    assert_eq!(long["days"], 240);
    assert_eq!(long["samples"], 1);
    assert_eq!(long["avg_return"], 30.0);
    assert_eq!(long["avg_excess_return"], 0.0);
    assert_eq!(kd["horizons"][3]["samples"], 0);
    assert_eq!(item(&json, "52_week", "new_high")["occurrences"], 1);

    // 大盤基準：6 筆 5 日報酬平均 11/6、4 筆為正。
    assert_eq!(json["benchmark"][0]["samples"], 6);
    assert_eq!(json["benchmark"][0]["avg_return"], 1.83);
    assert_eq!(json["benchmark"][0]["win_rate"], 0.6667);
    assert_eq!(json["benchmark"][1]["samples"], 1);
    assert!(json["benchmark"][2]["avg_return"].is_null());
    assert_eq!(json["benchmark"][4]["days"], 240);
    assert_eq!(json["benchmark"][4]["samples"], 1);
    assert_eq!(json["benchmark"][4]["win_rate"], 1.0);

    // 產業篩選只影響訊號，不影響大盤基準；上櫃的基準只算上櫃。
    let (status, json) = get(
        "/api/v1/market/signals/performance?from=1991-09-25&to=1991-10-02&industry=%E5%8D%8A%E5%B0%8E%E9%AB%94%E6%A5%AD&kind=golden_cross",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(item(&json, "kd", "golden_cross")["occurrences"], 3);
    assert_eq!(json["benchmark"][0]["samples"], 6);
    assert_eq!(json["kind"], "golden_cross");
    let (status, json) =
        get("/api/v1/market/signals/performance?from=1991-09-25&to=1991-10-02&market=tpex").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let tpex = item(&json, "kd", "golden_cross");
    assert_eq!(tpex["occurrences"], 1);
    assert_eq!(tpex["horizons"][0]["avg_excess_return"], 0.0);
    assert_eq!(json["benchmark"][0]["samples"], 2);

    for (query, status) in [
        (
            "from=1991-10-03&to=1991-10-02",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "from=1988-01-01&to=1991-10-02",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("to=1991-1-02", StatusCode::UNPROCESSABLE_ENTITY),
        ("from=bad", StatusCode::UNPROCESSABLE_ENTITY),
        ("market=otc", StatusCode::UNPROCESSABLE_ENTITY),
        ("kind=breakout", StatusCode::UNPROCESSABLE_ENTITY),
        ("indicator=ma10", StatusCode::UNPROCESSABLE_ENTITY),
        (
            "industry=%E4%B8%8D%E5%AD%98%E5%9C%A8",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("to=1900-01-01", StatusCode::NOT_FOUND),
    ] {
        let (actual, json) = get(&format!("/api/v1/market/signals/performance?{query}")).await;
        assert_eq!(actual, status, "{query}: {json}");
    }

    cleanup(pool).await;
}

/// 由新到舊、附報價與之後報酬；篩選、limit 與 total；查無訊號、未知代號與參數驗證。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn stock_signals_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("stock_signals_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool).await;

    let (status, json) = get("/api/v1/stocks/79979PA/signals?date=1991-10-03&days=5").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["stock_symbol"], "79979PA");
    assert_eq!(json["from"], "1991-09-29");
    assert_eq!(json["to"], "1991-10-03");
    assert_eq!(json["adjusted"], true);
    assert_eq!(json["total"], 3);
    let items: Vec<(String, String)> = json["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|item| {
            (
                item["date"].as_str().unwrap_or_default().to_owned(),
                item["indicator"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        items,
        [
            ("1991-10-02".to_owned(), "kd".to_owned()),
            ("1991-10-01".to_owned(), "kd".to_owned()),
            ("1991-10-01".to_owned(), "52_week".to_owned()),
        ]
    );
    let first_day = &json["items"][1];
    assert_eq!(first_day["closing_price"], 50.0);
    assert_eq!(first_day["change_percent"], 1.5);
    assert_eq!(first_day["return_5d"], 4.0);
    assert_eq!(first_day["return_20d"], 10.0);
    assert!(first_day["return_60d"].is_null());
    assert!(first_day["return_120d"].is_null());
    assert_eq!(first_day["return_240d"], 30.0);
    assert_eq!(first_day["description"], "79979PA golden_cross");
    assert!(json["items"][0]["closing_price"].is_null());

    let (_, json) = get("/api/v1/stocks/79979PA/signals?date=1991-10-03&limit=1").await;
    assert_eq!(json["total"], 3);
    assert_eq!(json["items"].as_array().map(Vec::len), Some(1));
    let (_, json) =
        get("/api/v1/stocks/79979PA/signals?date=1991-10-03&kind=new_high&indicator=52_week").await;
    assert_eq!(json["total"], 1);
    let (status, json) = get("/api/v1/stocks/79979PA/signals?date=1991-10-03&days=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["total"], 0);
    assert_eq!(json["items"].as_array().map(Vec::len), Some(0));

    for (path, status) in [
        ("/api/v1/stocks/79979ZZ/signals", StatusCode::NOT_FOUND),
        (
            "/api/v1/stocks/79979PA/signals?days=0",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/api/v1/stocks/79979PA/signals?days=731",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/api/v1/stocks/79979PA/signals?limit=0",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/api/v1/stocks/79979PA/signals?kind=bad",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/api/v1/stocks/79979PA/signals?date=1991/10/03",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let (actual, json) = get(path).await;
        assert_eq!(actual, status, "{path}: {json}");
    }

    cleanup(pool).await;
}
