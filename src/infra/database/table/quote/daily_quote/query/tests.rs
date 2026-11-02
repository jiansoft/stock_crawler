use crate::infra::cache::SHARE;

use super::*;

/// 只算有成交（成交量大於 0）的列，依交易日與市場別分組。
/// 測試用假代號與歷史日期，結束時刪除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn fetch_market_traded_counts_counts_only_traded_rows() {
    dotenvy::dotenv().ok();
    let date = NaiveDate::from_ymd_opt(1999, 3, 17).unwrap();
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol IN ('79976', '79977', '79978')"#,
            r#"DELETE FROM stocks WHERE stock_symbol IN ('79976', '79977', '79978')"#,
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 fetch_market_traded_counts_counts_only_traded_rows：無資料庫連接");
        return;
    }
    cleanup().await;

    // 79976 上市有成交、79977 上櫃有成交、79978 上櫃零量補值列
    for (symbol, market_id, volume) in [("79976", 2, 1_000), ("79977", 4, 2_000), ("79978", 4, 0)] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id)
               VALUES ($1, $1, $1, $2)"#,
        )
        .bind(symbol)
        .bind(market_id)
        .execute(pool)
        .await
        .expect("插入股票母檔");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "TradingVolume")
               VALUES ($1, $2, 10, $3)"#,
        )
        .bind(date)
        .bind(symbol)
        .bind(volume)
        .execute(pool)
        .await
        .expect("插入報價");
    }

    let counts = fetch_market_traded_counts(date, date).await;
    cleanup().await;

    let mut counts = counts.expect("查詢有成交檔數");
    counts.sort_by_key(|(_, market_id, _)| *market_id);
    assert_eq!(counts, vec![(date, 2, 1), (date, 4, 1)]);
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_daily_quotes_by_date() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_daily_quotes_by_date");
    let date = NaiveDate::from_ymd_opt(2023, 7, 31);
    match fetch_daily_quotes_by_date(date.unwrap()).await {
        Ok(dqs) => {
            tracing::debug!("fetch_daily_quotes_by_date: {:#?}", dqs);
        }
        Err(why) => {
            tracing::debug!("Failed to fetch_daily_quotes_by_date because {:?}", why);
        }
    }

    tracing::debug!("結束 fetch_daily_quotes_by_date");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_count_by_date() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_count_by_date");
    let date = NaiveDate::from_ymd_opt(2023, 7, 31);
    match fetch_count_by_date(date.unwrap()).await {
        Ok(count) => {
            tracing::debug!("count_by_date: {:?}", count);
        }
        Err(why) => {
            tracing::debug!("Failed to fetch_count_by_date because {:?}", why);
        }
    }

    tracing::debug!("結束 fetch_count_by_date");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_makeup_for_the_lack_daily_quotes() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    // 使用固定的歷史「真實交易日」驗證補列 SQL，避免以 Local::now()
    // （可能是週末或假日）為日期執行：當本機 .env 指向正式庫時，
    // 會整批複製前一交易日產生 0 成交量占位資料並污染正式資料
    // （2026-07-18 事件的根因之一）。歷史交易日的缺漏已補齊，
    // 重跑僅會插入 0 筆，冪等且無副作用。
    let now = NaiveDate::from_ymd_opt(2026, 4, 30).unwrap();

    tracing::debug!("開始 makeup_for_the_lack_daily_quotes");

    match makeup_for_the_lack_daily_quotes(now).await {
        Ok(result) => {
            tracing::debug!("result:{:#?}", result);
        }
        Err(why) => {
            tracing::debug!(
                "Failed to makeup_for_the_lack_daily_quotes because:{:?}",
                why
            );
        }
    }

    tracing::debug!("結束 makeup_for_the_lack_daily_quotes");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_lowest_avg_highest_price() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_lowest_avg_highest_price");

    match fetch_monthly_stock_price_summary("2330", 2023, 4).await {
        Ok(cd) => {
            tracing::debug!("stock: {:?}", cd);
        }
        Err(why) => {
            tracing::debug!("Failed to execute because {:?}", why);
        }
    }

    tracing::debug!("結束 fetch_lowest_avg_highest_price");
}
