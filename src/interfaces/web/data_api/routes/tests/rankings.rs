//! 殖利率排行 endpoint 的資料庫語意整合測試。

use axum::http::StatusCode;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::stocks::{get, pool_or_skip};

/// `(代號, 市場, 殖利率 %)`；79979YZ 市場別 0（如未歸類的 ETN），任何市場都不應納入。
const STOCKS: [(&str, i32, i64); 3] = [("79979YA", 2, 5), ("79979YB", 4, 3), ("79979YZ", 0, 9)];

/// 刪除種入的排行、股利、日報價與股票主檔。
async fn cleanup(pool: &sqlx::PgPool) {
    let symbols: Vec<&str> = STOCKS.iter().map(|row| row.0).collect();
    for sql in [
        "DELETE FROM yield_rank WHERE security_code = ANY($1)",
        "DELETE FROM dividend WHERE security_code = ANY($1)",
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(&symbols).execute(pool).await;
    }
}

/// 回傳排行中屬於本測試種入的代號（依排行順序）。
fn seeded_symbols(json: &serde_json::Value) -> Vec<String> {
    json["stocks"]
        .as_array()
        .expect("stocks array")
        .iter()
        .filter_map(|item| item["stock_symbol"].as_str())
        .filter(|symbol| symbol.starts_with("79979Y"))
        .map(str::to_owned)
        .collect()
}

/// 依殖利率排序、市場過濾，以及 all 不含市場別 0 的標的。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn dividend_yield_ranking_excludes_unclassified_market() {
    let Some(pool) = pool_or_skip("dividend_yield_ranking_excludes_unclassified_market").await
    else {
        return;
    };
    cleanup(pool).await;
    let date = NaiveDate::from_ymd_opt(1989, 7, 3).expect("固定日期合法");
    for (symbol, market_id, yield_percent) in STOCKS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, $1, $1, $2, 1)"#,
        )
        .bind(symbol)
        .bind(market_id)
        .execute(pool)
        .await
        .expect("種入 stocks");
        let (quote_serial,): (i64,) = sqlx::query_as(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice")
               VALUES ($1, $2, 100) RETURNING "Serial""#,
        )
        .bind(date)
        .bind(symbol)
        .fetch_one(pool)
        .await
        .expect("種入日報價");
        let (dividend_serial,): (i64,) = sqlx::query_as(
            r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, "sum")
               VALUES ($1, 1989, 1988, '', $2, $2) RETURNING serial"#,
        )
        .bind(symbol)
        .bind(Decimal::from(yield_percent))
        .fetch_one(pool)
        .await
        .expect("種入股利");
        sqlx::query(
            "INSERT INTO yield_rank (date, security_code, daily_quotes_serial, dividend_serial, yield)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(date)
        .bind(symbol)
        .bind(quote_serial)
        .bind(dividend_serial)
        .bind(Decimal::from(yield_percent))
        .execute(pool)
        .await
        .expect("種入殖利率排行");
    }

    let (status, json) =
        get("/api/v1/market/dividend-yield-ranking?date=1989-07-03&limit=50").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data_as_of"], "1989-07-03");
    assert_eq!(seeded_symbols(&json), ["79979YA", "79979YB"]);

    let (_, json) =
        get("/api/v1/market/dividend-yield-ranking?date=1989-07-03&market=tpex&limit=50").await;
    assert_eq!(seeded_symbols(&json), ["79979YB"]);

    cleanup(pool).await;
}
