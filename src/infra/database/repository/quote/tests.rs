use super::*;
use crate::domain::quote::entity::LastDailyQuote;
use chrono::Datelike;
use rust_decimal_macros::dec;

mod round_trip;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_cache_aside_flow() {
    // 載入環境變數設定
    dotenvy::dotenv().ok();

    if database::ping().await.is_err() {
        println!("跳過 test_cache_aside_flow：無資料庫連接");
        return;
    }

    if CLIENT.ping().await.is_err() {
        println!("跳過 test_cache_aside_flow：無 Redis 連接");
        return;
    }

    // 建立測試專用的倉儲對象
    let repo = PgQuoteRepository::new();
    let test_symbol = "TEST_9999";

    // 先取得快取的 key
    let cache_key = repo.get_cache_key(test_symbol);
    // 清除先前殘留的 Redis 快取以確保測試獨立性
    let _ = CLIENT.delete(&cache_key).await;

    // 建立測試用的最新收盤價領域對象
    let test_quote = LastDailyQuote {
        date: NaiveDate::from_ymd_opt(2026, 6, 8).unwrap(),
        stock_symbol: test_symbol.to_string(),
        closing_price: dec!(99.9),
    };

    // 1. 執行批次寫入，內部會同時寫入資料庫與 Redis 快取
    repo.save_last_quotes_batch(std::slice::from_ref(&test_quote))
        .await
        .unwrap();

    // 2. 第一次讀取，此時預期可以直接從快取中命中（因為 batch 寫入時有回寫）
    let fetched_first = repo.fetch_last_quote(test_symbol).await.unwrap();
    assert!(fetched_first.is_some());
    let fetched_first = fetched_first.unwrap();
    assert_eq!(fetched_first.stock_symbol, test_symbol);
    assert_eq!(fetched_first.closing_price, dec!(99.9));

    // 3. 再次清除快取以模擬 Cache Miss 的情境
    let _ = CLIENT.delete(&cache_key).await;

    // 4. 第二次讀取，因快取已被清除，會觸發 Cache Miss 降級並從 PostgreSQL 重新查詢，最後會回寫快取
    let fetched_miss = repo.fetch_last_quote(test_symbol).await.unwrap();
    assert!(fetched_miss.is_some());
    assert_eq!(fetched_miss.unwrap().closing_price, dec!(99.9));

    // 5. 驗證此時 Redis 快取是否已正確被自動回寫
    let redis_val = CLIENT.get_string(&cache_key).await.unwrap();
    let redis_quote: LastDailyQuote = serde_json::from_str(&redis_val).unwrap();
    assert_eq!(redis_quote.stock_symbol, test_symbol);
    assert_eq!(redis_quote.closing_price, dec!(99.9));

    // 6. 清理測試資料（同時清除 Redis 快取與資料庫內的測試列）
    let _ = CLIENT.delete(&cache_key).await;
    let _ = sqlx::query("DELETE FROM last_daily_quotes WHERE stock_symbol = $1")
        .bind(test_symbol)
        .execute(database::get_connection())
        .await;
}

/// 空清單在轉換與碰資料庫之前就短路回 0。
///
/// 回補流程對「這批沒有缺口」的情況會傳空清單進來，屬於正常路徑。
#[tokio::test]
async fn insert_missing_daily_quotes_short_circuits_on_an_empty_slice() {
    let repository = PgQuoteRepository::new();

    assert_eq!(
        repository
            .insert_missing_daily_quotes(&[])
            .await
            .expect("空清單應成功"),
        0
    );
}

/// 經由倉儲介面補寫缺口：領域實體正確轉成 Table 模型，且只補空位。
///
/// 與 table 層的同名測試分工不同：這裡驗證的是 domain → table 的轉換有沒有
/// 把日期與代號帶到位，寫入語意本身由 table 層那支負責。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn insert_missing_daily_quotes_maps_and_fills_gaps() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 insert_missing_daily_quotes_maps_and_fills_gaps：無資料庫連接");
        return;
    }

    // 固定歷史日期 + 假代號，避免碰到任何真實資料。
    const SYMBOL: &str = "79978";
    let day1 = NaiveDate::from_ymd_opt(1970, 1, 1).expect("測試日期應合法");
    let day2 = NaiveDate::from_ymd_opt(1970, 1, 2).expect("測試日期應合法");
    let cleanup = || async {
        let _ = sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE "stock_symbol" = $1"#)
            .bind(SYMBOL)
            .execute(database::get_connection())
            .await;
    };
    cleanup().await;

    let quote = |date: NaiveDate| DomainDailyQuote {
        stock_symbol: SYMBOL.to_string(),
        date,
        year: date.year(),
        month: date.month() as i32,
        day: date.day() as i32,
        closing_price: dec!(10),
        ..Default::default()
    };

    let repository = PgQuoteRepository::new();
    assert_eq!(
        repository
            .insert_missing_daily_quotes(&[quote(day1), quote(day2)])
            .await
            .expect("首次寫入應成功"),
        2
    );
    // 重跑不得產生重複資料列。
    assert_eq!(
        repository
            .insert_missing_daily_quotes(&[quote(day1), quote(day2)])
            .await
            .expect("重跑應成功"),
        0
    );

    // 轉換有把代號與日期帶到位，資料才查得回來。
    let stored: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM "DailyQuotes" WHERE "stock_symbol" = $1 AND "Date" IN ($2, $3)"#,
    )
    .bind(SYMBOL)
    .bind(day1)
    .bind(day2)
    .fetch_one(database::get_connection())
    .await
    .expect("應查得到寫入的資料");
    assert_eq!(stored, 2);

    cleanup().await;
}
