//! 產業表現 endpoint 的資料庫語意整合測試。

use axum::http::StatusCode;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::support::{get, pool_or_skip};

const INDUSTRY: i32 = 97_979;
/// `(代號, 市場, 發行股數)`；79979IZ 市場別 0（如未歸類的 ETN），任何市場都不應納入。
const STOCKS: [(&str, i32, i64); 3] = [
    ("79979IA", 2, 1_000),
    ("79979IB", 4, 2_000),
    ("79979IZ", 0, 0),
];

/// 固定的歷史交易日：區間起點、中間一天、資料日。
fn days() -> [NaiveDate; 3] {
    [(1989, 5, 31), (1989, 6, 1), (1989, 6, 2)]
        .map(|(y, m, d)| NaiveDate::from_ymd_opt(y, m, d).expect("固定日期合法"))
}

/// 刪除種入的日報價與股票主檔。
async fn cleanup(pool: &sqlx::PgPool) {
    let symbols: Vec<&str> = STOCKS.iter().map(|row| row.0).collect();
    for sql in [
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(&symbols).execute(pool).await;
    }
}

/// 種入日報價：`(日期, 代號, 收盤價, 漲跌幅 %, 成交金額)`。
async fn insert_quote(
    pool: &sqlx::PgPool,
    date: NaiveDate,
    symbol: &str,
    close: i64,
    change: i64,
    value: i64,
) {
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "ChangeRange", "TradeValue",
                                      "PriceEarningRatio", "price-to-book_ratio")
           VALUES ($1, $2, $3, $4, $5, 12, 0)"#,
    )
    .bind(date)
    .bind(symbol)
    .bind(Decimal::from(close))
    .bind(Decimal::from(change))
    .bind(Decimal::from(value))
    .execute(pool)
    .await
    .expect("種入日報價");
}

/// 找出測試產業那一列。
fn industry(json: &serde_json::Value) -> &serde_json::Value {
    json["industries"]
        .as_array()
        .expect("industries array")
        .iter()
        .find(|item| item["industry_id"] == INDUSTRY)
        .expect("回應應包含測試產業")
}

/// 單日與區間漲跌、市場過濾、休市日回推、參數驗證與查無資料。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn industry_performance_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("industry_performance_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    for (symbol, market_id, issued) in STOCKS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id, issued_share)
               VALUES ($1, $1, $1, $2, $3, $4)"#,
        )
        .bind(symbol)
        .bind(market_id)
        .bind(INDUSTRY)
        .bind(issued)
        .execute(pool)
        .await
        .expect("種入 stocks");
    }
    let [base, middle, last] = days();
    insert_quote(pool, base, "79979IA", 100, 0, 1_000).await;
    insert_quote(pool, base, "79979IB", 50, 0, 1_000).await;
    insert_quote(pool, middle, "79979IA", 105, 5, 1_000).await;
    insert_quote(pool, last, "79979IA", 110, 2, 3_000).await;
    insert_quote(pool, last, "79979IB", 45, -1, 1_000).await;
    insert_quote(pool, last, "79979IZ", 10, 9, 1_000).await;

    // 1989-06-03 是週六，回推到 06-02；單日用交易所漲跌幅；市場別 0 的 79979IZ 不納入。
    let (status, json) = get("/api/v1/market/industries?date=1989-06-03").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data_as_of"], "1989-06-02");
    assert_eq!(json["base_date"], "1989-06-01");
    assert_eq!(
        (json["market"].as_str(), json["days"].as_u64()),
        (Some("all"), Some(1))
    );
    let item = industry(&json);
    assert_eq!(item["stocks"], 2);
    assert_eq!(
        (item["advancers"].as_u64(), item["decliners"].as_u64()),
        (Some(1), Some(1))
    );
    assert_eq!(item["median_change_percent"], 0.5);
    assert_eq!(item["trade_value"], 4_000.0);
    assert_eq!(item["median_price_earning_ratio"], 12.0);
    assert!(item["median_price_to_book_ratio"].is_null());
    assert_eq!(item["top_gainer"]["stock_symbol"], "79979IA");
    assert_eq!(item["top_loser"]["change_percent"], -1.0);

    // 區間 2 天：期初 05-31，79979IA +10%、79979IB -10%；期初市值 100000 與 100000 → 加權 0%。
    let (status, json) = get("/api/v1/market/industries?date=1989-06-02&days=2").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["base_date"], "1989-05-31");
    let item = industry(&json);
    assert_eq!(item["top_gainer"]["change_percent"], 10.0);
    assert_eq!(item["top_loser"]["change_percent"], -10.0);
    assert_eq!(item["cap_weighted_change_percent"], 0.0);

    // 上櫃只剩 79979IB。
    let (_, json) = get("/api/v1/market/industries?date=1989-06-02&market=tpex").await;
    let item = industry(&json);
    assert_eq!(item["stocks"], 1);
    assert_eq!(item["top_loser"]["stock_symbol"], "79979IB");

    for query in ["days=0", "days=61", "market=otc", "date=1989-6-2"] {
        let (status, _) = get(&format!("/api/v1/market/industries?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }
    let (status, _) = get("/api/v1/market/industries?date=1900-01-01").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    cleanup(pool).await;
}
