//! 殖利率排行倉儲的測試。

use super::*;
use rust_decimal::Decimal;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_pg_yield_rank_repository_contract() {
    // 這是一個單元合約測試佔位符，如果沒有資料庫連接則跳過
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 PgYieldRankRepository DB 整合測試：無資料庫連接");
        return;
    }

    let repo = PgYieldRankRepository::new();
    let test_date = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();

    // 呼叫重建函式驗證基本 SQL 是否正常執行
    let result = repo.rebuild_by_date(test_date).await;
    if let Err(ref e) = result {
        println!("Error during rebuild_by_date: {:?}", e);
    }
    assert!(result.is_ok());
}

const RIGHTS_SYMBOLS: [&str; 4] = ["79979YA", "79979YB", "79979YC", "79979YD"];

/// 刪除配股殖利率測試種入的排行、股利、日線與股票主檔。
async fn cleanup_rights(pool: &sqlx::PgPool) {
    for sql in [
        "DELETE FROM yield_rank WHERE security_code = ANY($1)",
        "DELETE FROM dividend WHERE security_code = ANY($1)",
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(RIGHTS_SYMBOLS).execute(pool).await;
    }
}

/// 年度股利合計 6 元（其中股票股利 2 元、股數 × 1.2）、1988-08-10 收盤 50 元：
/// YA 已在 07-01 除權，殖利率 6 ÷ 1.2 ÷ 50 = 10%；YB 09-01 才除權，仍是 12%；
/// YC 除權日未定（`-`），不攤薄；YD 沒有配股，6 元現金為 12%。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_rebuild_by_date_dilutes_dividend_after_ex_rights() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_rebuild_by_date_dilutes_dividend_after_ex_rights：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    cleanup_rights(pool).await;
    for (symbol, cash, stock, ex_rights) in [
        ("79979YA", 4, 2, "1988-07-01"),
        ("79979YB", 4, 2, "1988-09-01"),
        ("79979YC", 4, 2, "-"),
        ("79979YD", 6, 0, "-"),
    ] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, $1, $1, 2, 99)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
        sqlx::query(
            r#"INSERT INTO dividend (security_code, "year", year_of_dividend, quarter, cash_dividend, stock_dividend,
                                     "sum", "ex-dividend_date1", "ex-dividend_date2")
               VALUES ($1, 1988, 1987, '', $2, $3, $2 + $3, '1988-06-01', $4)"#,
        )
        .bind(symbol)
        .bind(Decimal::from(cash))
        .bind(Decimal::from(stock))
        .bind(ex_rights)
        .execute(pool)
        .await
        .expect("種入股利");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice") VALUES ('1988-08-10', $1, 50)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入收盤");
    }

    let date = NaiveDate::from_ymd_opt(1988, 8, 10).expect("固定日期合法");
    let result = PgYieldRankRepository::new().rebuild_by_date(date).await;
    let rows: Vec<(String, f64)> = sqlx::query_as(
        "SELECT security_code, yield::float8 FROM yield_rank
         WHERE date = $1 AND security_code = ANY($2) ORDER BY security_code",
    )
    .bind(date)
    .bind(RIGHTS_SYMBOLS)
    .fetch_all(pool)
    .await
    .expect("讀取殖利率排行");
    cleanup_rights(pool).await;

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        rows,
        [
            ("79979YA".to_owned(), 10.0),
            ("79979YB".to_owned(), 12.0),
            ("79979YC".to_owned(), 12.0),
            ("79979YD".to_owned(), 12.0),
        ]
    );
}
