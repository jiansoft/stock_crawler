//! 技術訊號選股 endpoint 的資料庫語意整合測試。

use axum::http::StatusCode;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::support::{get, pool_or_skip};

/// `(代號, 市場, 產業, 成交金額)`；產業 24 為 CI 種入的「半導體業」。
const STOCKS: [(&str, i32, i32, i64); 2] = [("79979GA", 2, 24, 900), ("79979GB", 4, 99, 100)];

/// `(代號, 來源, 種類)`。
const SIGNALS: [(&str, &str, &str); 3] = [
    ("79979GA", "kd", "golden_cross"),
    ("79979GA", "52_week", "new_high"),
    ("79979GB", "kd", "golden_cross"),
];

fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(1991, 9, 3).expect("固定日期合法")
}

/// 刪除種入的訊號、日報價與股票主檔。
async fn cleanup(pool: &sqlx::PgPool) {
    let symbols: Vec<&str> = STOCKS.iter().map(|row| row.0).collect();
    for sql in [
        "DELETE FROM daily_technical_signal WHERE stock_symbol = ANY($1)",
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(&symbols).execute(pool).await;
    }
}

fn symbols(json: &serde_json::Value) -> Vec<(String, String)> {
    json["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|item| {
            (
                item["stock_symbol"].as_str().unwrap_or_default().to_owned(),
                item["kind"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// 休市日回推、種類與市場與產業篩選、成交值排序、參數驗證與查無資料。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn market_signals_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("market_signals_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    for (symbol, market_id, industry, value) in STOCKS {
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
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "ChangeRange", "TradeValue")
               VALUES ($1, $2, 50, 2.5, $3)"#,
        )
        .bind(date())
        .bind(symbol)
        .bind(Decimal::from(value))
        .execute(pool)
        .await
        .expect("種入日報價");
    }
    for (symbol, indicator, kind) in SIGNALS {
        sqlx::query(
            "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
             VALUES ($1, $2, $3, $4, $2 || ' ' || $4)",
        )
        .bind(date())
        .bind(symbol)
        .bind(indicator)
        .bind(kind)
        .execute(pool)
        .await
        .expect("種入訊號");
    }

    // 09-04 沒有訊號，回推到 09-03；依成交值排序。
    let (status, json) = get("/api/v1/market/signals?date=1991-09-04").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data_as_of"], "1991-09-03");
    assert_eq!(json["market"], "all");
    assert_eq!(json["adjusted"], false);
    assert_eq!(json["total"], 3);
    assert_eq!(
        symbols(&json),
        [
            ("79979GA".to_owned(), "golden_cross".to_owned()),
            ("79979GA".to_owned(), "new_high".to_owned()),
            ("79979GB".to_owned(), "golden_cross".to_owned()),
        ]
    );
    let first = &json["items"][0];
    assert_eq!(first["industry"], "半導體業");
    assert_eq!(first["market"], "twse");
    assert_eq!(first["closing_price"], 50.0);
    assert_eq!(first["change_percent"], 2.5);
    assert_eq!(first["trade_value"], 900.0);
    assert_eq!(first["description"], "79979GA golden_cross");
    assert_eq!(json["summary"][0]["indicator"], "kd");
    assert_eq!(json["summary"][0]["count"], 2);

    // 種類篩選與 limit；summary 不受種類影響。
    let (status, json) =
        get("/api/v1/market/signals?date=1991-09-03&kind=golden_cross&limit=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["total"], 2);
    assert_eq!(
        symbols(&json),
        [("79979GA".to_owned(), "golden_cross".to_owned())]
    );
    assert_eq!(json["summary"].as_array().expect("summary").len(), 2);

    // 市場、來源與產業篩選。
    let (_, json) = get("/api/v1/market/signals?date=1991-09-03&market=tpex").await;
    assert_eq!(
        symbols(&json),
        [("79979GB".to_owned(), "golden_cross".to_owned())]
    );
    assert_eq!(json["items"][0]["market"], "tpex");
    let (_, json) = get("/api/v1/market/signals?date=1991-09-03&indicator=52_week").await;
    assert_eq!(
        symbols(&json),
        [("79979GA".to_owned(), "new_high".to_owned())]
    );
    let (status, json) =
        get("/api/v1/market/signals?date=1991-09-03&industry=%E5%8D%8A%E5%B0%8E%E9%AB%94%E6%A5%AD")
            .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["industry"], "半導體業");
    assert_eq!(json["total"], 2);

    // 參數驗證與查無資料。
    for query in [
        "kind=unknown",
        "indicator=ma1",
        "limit=0",
        "limit=201",
        "market=otc",
        "date=1991-9-3",
        "industry=%E4%B8%8D%E5%AD%98%E5%9C%A8",
    ] {
        let (status, json) = get(&format!("/api/v1/market/signals?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}: {json}");
    }
    let (status, _) = get("/api/v1/market/signals?date=1900-01-01").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    cleanup(pool).await;
}
