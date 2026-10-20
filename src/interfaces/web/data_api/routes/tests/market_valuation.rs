//! `/market/valuation` 的資料庫語意整合測試。

use axum::http::StatusCode;

use super::support::{get, pool_or_skip};

/// 測試用假代號：VA、VB 上市，VC 上櫃。
const SYMBOLS: [(&str, i32); 3] = [("79979VA", 2), ("79979VB", 2), ("79979VC", 4)];

async fn cleanup(pool: &sqlx::PgPool) {
    let symbols: Vec<&str> = SYMBOLS.iter().map(|row| row.0).collect();
    for sql in [
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(&symbols).execute(pool).await;
    }
}

/// 1986-12-31、1987-01-30、1987-02-27 三檔都有資料；1987-03-02 只有 VA（樣本不足，不列入）。
/// VC 在 01-30 虧損（本益比 0）。
async fn seed(pool: &sqlx::PgPool) {
    for (symbol, market_id) in SYMBOLS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, $1, $1, $2, 1)"#,
        )
        .bind(symbol)
        .bind(market_id)
        .execute(pool)
        .await
        .expect("種入股票");
    }
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "PriceEarningRatio", "price-to-book_ratio") VALUES
           ('1986-12-31', '79979VA', 10, 1), ('1986-12-31', '79979VB', 20, 2), ('1986-12-31', '79979VC', 30, 3),
           ('1987-01-30', '79979VA', 12, 1.2), ('1987-01-30', '79979VB', 16, 1.6), ('1987-01-30', '79979VC', 0, 4),
           ('1987-02-27', '79979VA', 14, 1.4), ('1987-02-27', '79979VB', 18, 1.8), ('1987-02-27', '79979VC', 40, 5),
           ('1987-03-02', '79979VA', 99, 9)"#,
    )
    .execute(pool)
    .await
    .expect("種入日線本益比");
}

/// 每日中位數只計大於 0 的值、樣本不足的日子不列入、月底取樣、百分位與市場篩選；參數錯誤回 422，
/// 期間沒有資料回 404。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn market_valuation_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("market_valuation_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool).await;

    let (status, json) = get("/api/v1/market/valuation?date=1987-03-05&years=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    // 03-02 只有 1 檔，不到典型 3 檔的一半，最新一天是 02-27。
    assert_eq!(json["data_as_of"], "1987-02-27");
    assert_eq!(json["from"], "1986-12-31");
    assert_eq!(
        (json["years"].clone(), json["market"].clone()),
        (1.into(), "all".into())
    );
    // 本益比中位數：12-31 20、01-30 14（VC 虧損不計）、02-27 18；最新 18 落在第 2 高 → 2/3。
    assert_eq!(json["current"]["median_pe"], 18.0);
    assert_eq!(json["current"]["pe_samples"], 3);
    assert_eq!(json["pe"]["percentile"], 66.67);
    assert_eq!(
        (
            json["pe"]["min"].clone(),
            json["pe"]["max"].clone(),
            json["pe"]["days"].clone()
        ),
        (14.0.into(), 20.0.into(), 3.into())
    );
    assert_eq!(json["pb"]["current"], 1.8);
    let months: Vec<&str> = json["monthly"]
        .as_array()
        .expect("monthly")
        .iter()
        .filter_map(|point| point["date"].as_str())
        .collect();
    assert_eq!(months, ["1986-12-31", "1987-01-30", "1987-02-27"]);
    assert_eq!(json["monthly"][1]["pe_samples"], 2);

    // 只看上櫃：只有 VC，12-31 30、02-27 40（01-30 虧損，當天只有淨值比）。
    let (status, json) = get("/api/v1/market/valuation?date=1987-03-05&years=1&market=tpex").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["current"]["median_pe"], 40.0);
    assert_eq!(json["pe"]["percentile"], 100.0);
    assert!(json["monthly"][1]["median_pe"].is_null(), "{json}");

    let (status, _) = get("/api/v1/market/valuation?date=1980-01-01&years=1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for bad in ["years=0", "years=11", "market=hk", "date=1987-3-5"] {
        let (status, _) = get(&format!("/api/v1/market/valuation?{bad}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }

    cleanup(pool).await;
}
