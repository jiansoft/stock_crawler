//! 每日股價統計（市場廣度、52 週新高新低）寫入與查詢的測試。

use crate::infra::cache::SHARE;
use std::time::Duration;
use tokio::time::sleep;

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_upsert() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 DailyStockPriceStats::upsert");

    // 開始日期與結束日期
    let start_date = NaiveDate::parse_from_str("2026-02-03", "%Y-%m-%d").unwrap();
    let end_date = NaiveDate::parse_from_str("2026-02-03", "%Y-%m-%d").unwrap();

    // 迴圈遍歷日期
    let mut current_date = start_date;
    while current_date <= end_date {
        tracing::debug!("處理日期: {}", current_date);

        match DailyStockPriceStats::upsert(current_date, &mut None).await {
            Ok(r) => {
                tracing::debug!(
                    "DailyStockPriceStats::upsert({:?}) 成功: {:#?}",
                    current_date,
                    r
                );
            }
            Err(why) => {
                tracing::debug!(
                    "DailyStockPriceStats::upsert({:?}) 失敗: {:?}",
                    current_date,
                    why
                );
            }
        }

        // 日期加一天
        current_date += chrono::Duration::days(1);
    }

    tracing::debug!("結束 DailyStockPriceStats::upsert");
    // 每次迴圈暫停 0.5 秒
    sleep(Duration::from_millis(500)).await;
}
/// 52 週新高／新低：79979HA 創高、79979HB 創低、79979HC 歷史不足 200 日不計；
/// 上市列與 all 列都只有 1 檔新高、1 檔新低；興櫃 79979HD 創高只算在興櫃列。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn upsert_counts_52_week_highs_and_lows() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 upsert_counts_52_week_highs_and_lows：無資料庫連接");
        return;
    }
    let date = NaiveDate::from_ymd_opt(1991, 6, 28).expect("固定日期合法");
    let symbols = ["79979HA", "79979HB", "79979HC", "79979HD"];
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
            "DELETE FROM estimate WHERE security_code = ANY($1)",
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
        ] {
            let _ = sqlx::query(sql).bind(symbols).execute(pool).await;
        }
        let _ = sqlx::query("DELETE FROM daily_stock_price_stats WHERE date = $1")
            .bind(date)
            .execute(pool)
            .await;
    };
    cleanup().await;

    // (代號, 市場, 前期天數, 當日最高, 當日最低)；前期每日最高 100、最低 90。
    // 79979HD 是興櫃：只計入自己的分項列，不計入 all（id 0）。
    for (symbol, market_id, prior_days, high, low) in [
        ("79979HA", 2, 250, 105, 95),
        ("79979HB", 2, 250, 95, 85),
        ("79979HC", 2, 50, 200, 150),
        ("79979HD", 5, 250, 105, 95),
    ] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, $1, $1, $2, 1)"#,
        )
        .bind(symbol)
        .bind(market_id)
        .execute(pool)
        .await
        .expect("種入 stocks");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "HighestPrice", "LowestPrice")
               SELECT $2::date - g, $1, 95, 100, 90 FROM generate_series(1, $3) g
               UNION ALL SELECT $2::date, $1, $4, $4, $5"#,
        )
        .bind(symbol)
        .bind(date)
        .bind(prior_days)
        .bind(high)
        .bind(low)
        .execute(pool)
        .await
        .expect("種入日報價");
        sqlx::query("INSERT INTO estimate (security_code, date) VALUES ($1, $2)")
            .bind(symbol)
            .bind(date)
            .execute(pool)
            .await
            .expect("種入 estimate");
    }

    DailyStockPriceStats::upsert(date, &mut None)
        .await
        .expect("upsert 成功");
    let rows: Vec<(i32, i32, i32)> = sqlx::query_as(
        "SELECT stock_exchange_market_id, new_52_week_high, new_52_week_low
         FROM daily_stock_price_stats WHERE date = $1 ORDER BY 1",
    )
    .bind(date)
    .fetch_all(pool)
    .await
    .expect("讀回統計");
    assert_eq!(rows, [(0, 1, 1), (2, 1, 1), (5, 1, 0)]);

    cleanup().await;
}

/// 歷史市場廣度回補：沒有估價也算、含已下市股票、排除 `00` 開頭的 ETF；估值四欄為 0；
/// 已有的統計列不覆寫。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn backfill_breadth_counts_without_estimate_and_keeps_existing_rows() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!(
            "跳過 backfill_breadth_counts_without_estimate_and_keeps_existing_rows：無資料庫連接"
        );
        return;
    }
    let date = NaiveDate::from_ymd_opt(1991, 7, 31).expect("固定日期合法");
    let symbols = ["79979HE", "79979HF", "0079979"];
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
        ] {
            let _ = sqlx::query(sql).bind(symbols).execute(pool).await;
        }
        let _ = sqlx::query("DELETE FROM daily_stock_price_stats WHERE date = $1")
            .bind(date)
            .execute(pool)
            .await;
    };
    cleanup().await;

    // (代號, 市場, 已下市, 收盤, 60 日均線, 漲跌幅)：79979HE 已下市仍要算；0079979 是 ETF 不算。
    for (symbol, market_id, suspended, close, ma60, change) in [
        ("79979HE", 2, true, 110, 100, 1),
        ("79979HF", 4, false, 90, 100, -1),
        ("0079979", 2, false, 120, 100, 1),
    ] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id, "SuspendListing")
               VALUES ($1, $1, $1, $2, 1, $3)"#,
        )
        .bind(symbol)
        .bind(market_id)
        .bind(suspended)
        .execute(pool)
        .await
        .expect("種入 stocks");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "HighestPrice", "LowestPrice", "MovingAverage60", "ChangeRange")
               VALUES ($2, $1, $3, $3, $3, $4, $5)"#,
        )
        .bind(symbol)
        .bind(date)
        .bind(close)
        .bind(ma60)
        .bind(change)
        .execute(pool)
        .await
        .expect("種入日報價");
    }

    let written = DailyStockPriceStats::backfill_breadth(date)
        .await
        .expect("回補成功");
    assert_eq!(written, 3, "all、上市、上櫃各一列");
    let rows: Vec<(i32, i32, i32, i32, i32, i32)> = sqlx::query_as(
        "SELECT stock_exchange_market_id, above_60_day_moving_average, below_60_day_moving_average,
                stocks_up, stocks_down, undervalued + fair_valued + overvalued + highly_overvalued
         FROM daily_stock_price_stats WHERE date = $1 ORDER BY 1",
    )
    .bind(date)
    .fetch_all(pool)
    .await
    .expect("讀回統計");
    assert_eq!(
        rows,
        [(0, 1, 1, 1, 1, 0), (2, 1, 0, 1, 0, 0), (4, 0, 1, 0, 1, 0)]
    );

    // 已有的列不覆寫：手動改掉 all 列後再回補一次，不寫入也不還原。
    sqlx::query(
        "UPDATE daily_stock_price_stats SET above_60_day_moving_average = 99
         WHERE date = $1 AND stock_exchange_market_id = 0",
    )
    .bind(date)
    .execute(pool)
    .await
    .expect("改寫統計");
    let written = DailyStockPriceStats::backfill_breadth(date)
        .await
        .expect("第二次回補成功");
    assert_eq!(written, 0);
    let kept: i32 = sqlx::query_scalar(
        "SELECT above_60_day_moving_average FROM daily_stock_price_stats
         WHERE date = $1 AND stock_exchange_market_id = 0",
    )
    .bind(date)
    .fetch_one(pool)
    .await
    .expect("讀回 all 列");
    assert_eq!(kept, 99);

    // 沒有日線的日子（非交易日）不寫入，連全 0 的合併列也不產生。
    let holiday = date + chrono::Duration::days(3);
    let written = DailyStockPriceStats::backfill_breadth(holiday)
        .await
        .expect("非交易日回補成功");
    assert_eq!(written, 0);

    cleanup().await;
}
