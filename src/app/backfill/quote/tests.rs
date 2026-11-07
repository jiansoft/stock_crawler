//! quote的測試；連外的測試標 #[ignore]。

/// 兩邊都有資料或都沒有（休市）才放行；只有一邊有資料時拒絕寫入。
#[test]
fn ensure_both_markets_rejects_a_single_market() {
    assert!(super::ensure_both_markets(1_050, 860).is_ok());
    assert!(super::ensure_both_markets(0, 0).is_ok());
    let err = super::ensure_both_markets(1_050, 0).expect_err("上櫃 0 筆");
    assert!(err.to_string().contains("上櫃收盤資料 0 筆"), "{err}");
    let err = super::ensure_both_markets(0, 860).expect_err("上市 0 筆");
    assert!(err.to_string().contains("上市收盤資料 0 筆"), "{err}");
}

/// 來源成功時原樣回傳；失敗時錯誤訊息帶來源名稱，原始錯誤留在 source chain。
#[tokio::test]
async fn get_quotes_from_source_names_the_failed_source() {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).expect("有效日期");
    let quotes = super::get_quotes_from_source(
        async move { Ok(vec![super::DailyQuoteDto::new("2330", date)]) },
        "上市",
    )
    .await
    .expect("來源成功");
    assert_eq!(quotes.len(), 1);

    let err = super::get_quotes_from_source(async { Err(anyhow::anyhow!("逾時")) }, "上櫃")
        .await
        .expect_err("來源失敗");
    assert_eq!(err.to_string(), "抓取上櫃收盤數據失敗");
    assert_eq!(err.root_cause().to_string(), "逾時");
}

/// 同一天再寫一次是整天替換：第二批沒有的代號會被刪掉。
/// 用週日（不會有真實行情）與假代號，結束時清除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn process_quotes_replaces_the_whole_day() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 process_quotes_replaces_the_whole_day：無資料庫連接");
        return;
    }
    let date = chrono::NaiveDate::from_ymd_opt(2001, 4, 1).expect("有效日期");
    let cleanup = || async {
        let _ = sqlx::query(
            r#"DELETE FROM "DailyQuotes" WHERE "Date" = $1 AND stock_symbol LIKE '79979Q%'"#,
        )
        .bind(date)
        .execute(pool)
        .await;
    };
    cleanup().await;
    let quote = |symbol: &str, close: i64| {
        let mut dto = super::DailyQuoteDto::new(symbol, date);
        let close = rust_decimal::Decimal::from(close);
        dto.opening_price = close;
        dto.highest_price = close;
        dto.lowest_price = close;
        dto.closing_price = close;
        dto
    };
    let rows = || async {
        sqlx::query_as::<_, (String, rust_decimal::Decimal)>(
            r#"SELECT stock_symbol, "ClosingPrice" FROM "DailyQuotes" WHERE "Date" = $1 ORDER BY stock_symbol"#,
        )
        .bind(date)
        .fetch_all(pool)
        .await
        .expect("查詢")
    };

    let written = super::process_quotes(date, vec![quote("79979QA", 10), quote("79979QB", 20)])
        .await
        .expect("第一次寫入");
    assert_eq!(written, 2);
    assert_eq!(rows().await.len(), 2);

    let written = super::process_quotes(date, vec![quote("79979QA", 11)])
        .await
        .expect("整天替換");
    assert_eq!(written, 1);
    assert_eq!(
        rows().await,
        vec![("79979QA".to_string(), rust_decimal::Decimal::from(11))]
    );

    cleanup().await;
}

use chrono::NaiveDate;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

//use crossbeam::thread;
use rayon::prelude::*;
use tokio::time::sleep;

use crate::infra::cache::SHARE;

use super::*;

//use std::time;

#[tokio::test]
#[ignore]
async fn test_execute() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 execute");
    //let date = Local::now().date_naive();
    let date = NaiveDate::from_ymd_opt(2026, 4, 30).unwrap();
    // execute 內部已改為「先抓取、後於單一 transaction 原子替換」，
    // 不需要（也不應該）在呼叫前先手動刪除當日資料。

    match execute(date).await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("Failed to execute because {:?}", why);
        }
    }

    tracing::debug!("結束 execute");
    sleep(Duration::from_secs(1)).await;
}

#[tokio::test]
#[ignore]
async fn test_thread() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 execute");

    let stock_repo = crate::infra::database::repository::stock::PgStockRepository::new();
    use crate::domain::registry::repository::StockRepository;
    let stocks = stock_repo.fetch_all_active().await.unwrap();
    let worker_count = num_cpus::get() * 100;
    //let dqs_arc = Arc::new(stocks);
    let counter = Arc::new(AtomicUsize::new(0));
    tracing::debug!("stocks:{}", stocks.len());
    /*  thread::scope(|scope| {
        for i in 0..worker_count {
            let dqs = Arc::clone(&dqs_arc);
            let counter = Arc::clone(&counter);

            scope.spawn(move |_| {
                calculate_day_quotes_moving_average_worker(i, dqs.to_vec(), &counter);
            });
        }
    })
    .unwrap();*/
    stocks
        .par_iter()
        .with_min_len(worker_count)
        .for_each(|stock| {
            let index = counter.fetch_add(1, Ordering::SeqCst);
            calculate_day_quotes_moving_average_worker(index, stock);
        });

    tracing::debug!("結束 execute");
    sleep(Duration::from_secs(1)).await;
}

fn calculate_day_quotes_moving_average_worker(
    i: usize,
    dq: &crate::domain::registry::entity::Stock,
) {
    tracing::debug!("dq[{}]:{:?}", i, dq);
}
