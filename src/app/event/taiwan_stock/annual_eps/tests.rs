//! 年度 EPS 補齊流程的測試：缺年報清單、Redis 略過旗標與實際抓取（連外，`#[ignore]`）。

use super::*;
use crate::domain::financial::repository::FinancialRepository;
use crate::infra::database;
use crate::infra::database::repository::financial::PgFinancialRepository;
use crate::infra::nosql::redis::CLIENT;

/// 對真實來源補齊一次（會連外並寫入資料庫）。
#[tokio::test]
#[ignore]
async fn test_execute() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 execute");

    match execute().await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("err:{:#?} ", why);
        }
    }

    tracing::debug!("結束 execute");
}

const SYMBOL: &str = "79977A";

async fn cleanup(pool: &sqlx::PgPool) {
    for sql in [
        "DELETE FROM financial_statement WHERE security_code = $1",
        "DELETE FROM stocks WHERE stock_symbol = $1",
    ] {
        let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
    }
    let _ = CLIENT
        .delete(&format!("financial_statement:annual:{SYMBOL}"))
        .await;
}

/// 缺年報的股票已有 Redis 略過旗標時不連外、不寫入，流程正常結束。
///
/// 只種一檔假代號（2024 年只有 Q1、沒有年報列）並先設旗標；資料庫裡若還有其他缺年報的股票，
/// 執行會連外抓取，因此直接略過，避免測試連外或改到真實代號的旗標。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL、Redis），請加 --features integration-tests 執行"
)]
async fn execute_skips_symbols_flagged_in_redis() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() || CLIENT.ping().await.is_err() {
        println!("跳過 execute_skips_symbols_flagged_in_redis：無資料庫或 Redis 連接");
        return;
    }
    cleanup(pool).await;
    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing") VALUES ($1, '年度 EPS 測試', $1, 2, false)"#,
    )
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("種入股票");
    sqlx::query(
        "INSERT INTO financial_statement (security_code, year, quarter, earnings_per_share) VALUES ($1, 2024, 'Q1', 1)",
    )
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("種入季報");

    let last_year = Local::now().date_naive().year() - 1;
    let missing = PgFinancialRepository::new()
        .fetch_without_annual_statements(last_year)
        .await
        .expect("查詢缺年報清單");
    let symbols: HashSet<&str> = missing
        .iter()
        .map(|statement| statement.security_code.as_str())
        .collect();
    assert!(
        symbols.contains(SYMBOL),
        "假代號應在缺年報清單：{symbols:?}"
    );
    if symbols.len() > 1 {
        println!(
            "跳過 execute_skips_symbols_flagged_in_redis：資料庫還有其他缺年報的股票 {symbols:?}"
        );
        cleanup(pool).await;
        return;
    }

    let cache_key = format!("financial_statement:annual:{SYMBOL}");
    CLIENT.set(&cache_key, true, 60).await.expect("設略過旗標");
    let result = execute().await;

    let annual: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM financial_statement WHERE security_code = $1 AND quarter = ''",
    )
    .bind(SYMBOL)
    .fetch_one(pool)
    .await
    .expect("查詢年報列");
    cleanup(pool).await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(annual, 0, "有略過旗標時不應抓取或寫入");
}
