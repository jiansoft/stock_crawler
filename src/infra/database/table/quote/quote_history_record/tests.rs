//! 歷史股價紀錄表的測試。

use core::result::Result::Ok;

use rust_decimal_macros::dec;

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_upsert() {
    dotenvy::dotenv().ok();
    tracing::info!("開始 upsert");
    let date = NaiveDate::from_ymd_opt(2023, 8, 2);
    let mut qhr = QuoteHistoryRecord::new("79979".to_string());
    qhr.maximum_price = dec!(1.1);
    qhr.maximum_price_date_on = date.unwrap();
    qhr.maximum_price_to_book_ratio = dec!(1.11);
    qhr.maximum_price_to_book_ratio_date_on = date.unwrap();
    qhr.minimum_price = dec!(2);
    qhr.minimum_price_date_on = date.unwrap();
    qhr.minimum_price_to_book_ratio = dec!(2.2);
    qhr.minimum_price_to_book_ratio_date_on = date.unwrap();

    match qhr.upsert().await {
        Ok(_) => tracing::info!("{:#?}", qhr),
        Err(why) => {
            tracing::error!("Failed to upsert because {:?}", why);
        }
    }

    // 測試結束後移除假代號資料列，避免測試資料殘留在資料庫
    // （曾在正式庫發現本測試留下的 '79979' 紀錄）。
    if let Err(why) = sqlx::query("DELETE FROM quote_history_record WHERE security_code = $1")
        .bind(&qhr.security_code)
        .execute(database::get_connection())
        .await
    {
        tracing::error!("Failed to cleanup test row because {:?}", why);
    }

    tracing::info!("結束 upsert");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch() {
    dotenvy::dotenv().ok();
    tracing::info!("開始 fetch");

    match QuoteHistoryRecord::fetch().await {
        Ok(qhr) => tracing::info!("{:#?}", qhr),
        Err(why) => {
            tracing::error!("Failed to fetch because {:?}", why);
        }
    }

    tracing::info!("結束 fetch");
}

/// 依日K重建最高、最低價：最低價略過 0（無成交），同價取最早一天；淨值比極值不動；
/// 重跑時沒有變化不寫入。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn rebuild_price_extremes_from_daily_quotes() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 rebuild_price_extremes_from_daily_quotes：無資料庫連接");
        return;
    }
    let symbol = "79979QH".to_string();
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
            "DELETE FROM quote_history_record WHERE security_code = $1",
        ] {
            sqlx::query(sql)
                .bind("79979QH")
                .execute(database::get_connection())
                .await
                .expect("清除測試資料");
        }
    };
    cleanup().await;

    // 01-02 與 01-05 同為最高 12；01-03 無成交（最低 0）；01-04 最低 9。
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" (stock_symbol, "Date", "HighestPrice", "LowestPrice", "ClosingPrice")
           VALUES ($1, '2001-01-02', 12, 10, 11), ($1, '2001-01-03', 0, 0, 0),
                  ($1, '2001-01-04', 11, 9, 10), ($1, '2001-01-05', 12, 10, 11)"#,
    )
    .bind(&symbol)
    .execute(database::get_connection())
    .await
    .expect("寫入測試日K");
    let mut old = QuoteHistoryRecord::new(symbol.clone());
    old.maximum_price = dec!(5);
    old.maximum_price_to_book_ratio = dec!(3.3);
    old.upsert().await.expect("寫入舊紀錄");

    let symbols = [symbol.clone()];
    assert_eq!(
        QuoteHistoryRecord::rebuild_price_extremes(&symbols)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        QuoteHistoryRecord::rebuild_price_extremes(&symbols)
            .await
            .unwrap(),
        0
    );

    let row: (Decimal, NaiveDate, Decimal, NaiveDate, Decimal) = sqlx::query_as(
        r#"SELECT maximum_price, maximum_price_date_on, minimum_price, minimum_price_date_on,
                  "maximum_price-to-book_ratio"
           FROM quote_history_record WHERE security_code = $1"#,
    )
    .bind(&symbol)
    .fetch_one(database::get_connection())
    .await
    .expect("讀回紀錄");
    cleanup().await;

    let day = |d| NaiveDate::from_ymd_opt(2001, 1, d).unwrap();
    assert_eq!(row, (dec!(12), day(2), dec!(9), day(4), dec!(3.3)));
}
