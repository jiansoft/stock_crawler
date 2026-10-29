//! 季報財報回補（Yahoo 補值、期別不符時略過、整體執行）的測試。

use std::sync::Mutex;

use anyhow::anyhow;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::{
    core::declare::Quarter,
    infra::{
        cache::SHARE, crawler::yahoo::dividend::YahooPageNotFoundError, database,
        nosql::redis::CLIENT,
    },
};

use super::*;

/// 假代號：C 補得到、D 的個股頁 404、E 是一般錯誤。
const SYMBOL_C: &str = "79979RC";
const SYMBOL_D: &str = "79979RD";
const SYMBOL_E: &str = "79979RE";
const SYMBOLS: [&str; 3] = [SYMBOL_C, SYMBOL_D, SYMBOL_E];
/// 固定的歷史季度，不使用今天的日期當資料日期。
const REPORT: ReportQuarter = ReportQuarter {
    year: 2020,
    quarter: Quarter::Q1,
};

fn success_cache_key(symbol: &str) -> String {
    format!("FinancialStatement:{symbol}-2020-Q1")
}

async fn cleanup() {
    let pool = database::get_connection();
    for symbol in SYMBOLS {
        for sql in [
            "DELETE FROM financial_statement WHERE security_code = $1",
            "DELETE FROM stocks WHERE stock_symbol = $1",
        ] {
            let _ = sqlx::query(sql).bind(symbol).execute(pool).await;
        }
        let _ = CLIENT.delete(&success_cache_key(symbol)).await;
        let _ = CLIENT
            .delete(&yahoo::profile::no_valid_data_cache_key(symbol))
            .await;
    }
}

/// 2020-Q1 ROE、ROA 都是 0 的財報。
fn zero_statement(symbol: &str) -> FinancialStatement {
    FinancialStatement {
        security_code: symbol.to_owned(),
        year: 2020,
        quarter: "Q1".to_owned(),
        net_asset_value_per_share: dec!(30),
        earnings_per_share: dec!(2),
        ..FinancialStatement::default()
    }
}

async fn pending_rows(repo: &PgFinancialRepository) -> Vec<FinancialStatement> {
    repo.fetch_roe_or_roa_equal_to_zero(Some(REPORT.year), Some(REPORT.quarter))
        .await
        .expect("fetch_roe_or_roa_equal_to_zero")
        .into_iter()
        .filter(|fs| SYMBOLS.contains(&fs.security_code.as_str()))
        .collect()
}

/// 補到的寫回並記成功快取；404 記排除快取；一般錯誤不記，下一輪只再查它。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn fill_from_yahoo_profile_saves_found_profiles_and_caches_outcomes() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() || CLIENT.ping().await.is_err() {
        println!(
            "跳過 fill_from_yahoo_profile_saves_found_profiles_and_caches_outcomes：無資料庫或 Redis 連接"
        );
        return;
    }
    cleanup().await;
    for symbol in SYMBOLS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing") VALUES ($1, 'profile 補值測試', $1, 2, false)"#,
        )
        .bind(symbol)
        .execute(database::get_connection())
        .await
        .expect("種入股票主檔");
    }
    let repo = PgFinancialRepository::new();
    repo.batch_save_financial_statements(&SYMBOLS.map(zero_statement))
        .await
        .expect("種入假代號財報");

    let calls = Mutex::new(Vec::new());
    let fetch = |symbol: String| {
        calls.lock().expect("calls lock").push(symbol.clone());
        async move {
            match symbol.as_str() {
                SYMBOL_C => Ok(yahoo::profile::Profile {
                    quarter: "Q1".to_owned(),
                    stock_symbol: symbol.clone(),
                    net_asset_value_per_share: dec!(31),
                    earnings_per_share: dec!(2),
                    return_on_equity: dec!(6),
                    return_on_assets: dec!(4),
                    year: 2020,
                    ..yahoo::profile::Profile::default()
                }),
                SYMBOL_D => Err(anyhow::Error::new(YahooPageNotFoundError {
                    stock_symbol: symbol.clone(),
                    url: "https://example.invalid".to_owned(),
                })),
                _ => Err(anyhow!("Yahoo 測試失敗")),
            }
        }
    };

    let saved = fill_from_yahoo_profile(&repo, REPORT, pending_rows(&repo).await, &fetch)
        .await
        .expect("fill_from_yahoo_profile");
    assert_eq!(saved, 1);
    let (roe, roa): (Decimal, Decimal) = sqlx::query_as(
        "SELECT return_on_equity, return_on_assets FROM financial_statement WHERE security_code = $1 AND year = 2020 AND quarter = 'Q1'",
    )
    .bind(SYMBOL_C)
    .fetch_one(database::get_connection())
    .await
    .expect("讀回假代號財報");
    assert_eq!((roe, roa), (dec!(6), dec!(4)));
    assert!(
        CLIENT
            .get_bool(&success_cache_key(SYMBOL_C))
            .await
            .unwrap_or(false)
    );
    assert!(
        CLIENT
            .get_bool(&yahoo::profile::no_valid_data_cache_key(SYMBOL_D))
            .await
            .unwrap_or(false)
    );
    assert!(
        !CLIENT
            .get_bool(&yahoo::profile::no_valid_data_cache_key(SYMBOL_E))
            .await
            .unwrap_or(false)
    );

    // 第二輪 C 已補好不在待補清單、D 被排除快取擋住，只剩 E 再查一次。
    calls.lock().expect("calls lock").clear();
    let saved = fill_from_yahoo_profile(&repo, REPORT, pending_rows(&repo).await, &fetch)
        .await
        .expect("fill_from_yahoo_profile");
    assert_eq!(saved, 0);
    assert_eq!(
        *calls.lock().expect("calls lock"),
        vec![SYMBOL_E.to_owned()]
    );

    cleanup().await;
}

/// Yahoo 回的是上一季：不寫入、不設目標季的完成旗標，下一輪照樣再查。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn fill_from_yahoo_profile_skips_profiles_of_another_quarter() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() || CLIENT.ping().await.is_err() {
        println!(
            "跳過 fill_from_yahoo_profile_skips_profiles_of_another_quarter：無資料庫或 Redis 連接"
        );
        return;
    }
    cleanup().await;
    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing") VALUES ($1, 'profile 期別測試', $1, 2, false)"#,
    )
    .bind(SYMBOL_C)
    .execute(database::get_connection())
    .await
    .expect("種入股票主檔");
    let repo = PgFinancialRepository::new();
    repo.batch_save_financial_statements(&[zero_statement(SYMBOL_C)])
        .await
        .expect("種入假代號財報");

    let calls = Mutex::new(0);
    let fetch = |symbol: String| {
        *calls.lock().expect("calls lock") += 1;
        async move {
            Ok(yahoo::profile::Profile {
                quarter: "Q4".to_owned(),
                stock_symbol: symbol,
                return_on_equity: dec!(6),
                return_on_assets: dec!(4),
                year: 2019,
                ..yahoo::profile::Profile::default()
            })
        }
    };

    for _ in 0..2 {
        let saved = fill_from_yahoo_profile(&repo, REPORT, pending_rows(&repo).await, &fetch)
            .await
            .expect("fill_from_yahoo_profile");
        assert_eq!(saved, 0);
    }
    assert_eq!(*calls.lock().expect("calls lock"), 2);
    assert!(
        !CLIENT
            .get_bool(&success_cache_key(SYMBOL_C))
            .await
            .unwrap_or(false)
    );
    let other: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM financial_statement WHERE security_code = $1 AND year = 2019",
    )
    .bind(SYMBOL_C)
    .fetch_one(database::get_connection())
    .await
    .expect("查上一季");
    assert_eq!(other, 0);

    cleanup().await;
}

#[tokio::test]
#[ignore]
async fn test_execute() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 execute");

    match execute().await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("Failed to execute because {:?}", why);
        }
    }

    tracing::debug!("結束 execute");
}
