//! 大盤未來報酬彙總與明細比對（[`MARKET_FORWARD_RETURN_DRIFT_SQL`]）的測試。

use super::*;

/// 彙總筆數與明細一致的日子不列；少算一筆、彙總少一天、彙總多一天都列出。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn market_forward_return_drift_lists_mismatched_dates() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 market_forward_return_drift_lists_mismatched_dates：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let mut tx = pool.begin().await.expect("開交易應成功");
    // 交易內清空，測完 rollback，不影響其他測試的資料。
    for sql in [
        "DELETE FROM daily_forward_return",
        "DELETE FROM daily_market_forward_return",
    ] {
        sqlx::query(sql)
            .execute(&mut *tx)
            .await
            .expect("清空應成功");
    }
    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing")
SELECT s, '彙總比對測試', s, 2, false FROM unnest(ARRAY['79979M1', '79979M2']) AS s
ON CONFLICT DO NOTHING"#,
    )
    .execute(&mut *tx)
    .await
    .expect("種入股票主檔");
    let day = |d| NaiveDate::from_ymd_opt(2021, 3, d).expect("日期應合法");
    for date in [day(1), day(2), day(3)] {
        sqlx::query(
            r#"INSERT INTO daily_forward_return (date, stock_symbol, return_5d, return_20d)
SELECT $1, s, 1.5, NULL FROM unnest(ARRAY['79979M1', '79979M2']) AS s"#,
        )
        .bind(date)
        .execute(&mut *tx)
        .await
        .expect("寫入明細應成功");
    }
    // 3-01 一致；3-02 少算一筆；3-03 彙總沒有；3-04 只有彙總。
    for (date, n5) in [(day(1), 2_i64), (day(2), 1), (day(4), 2)] {
        sqlx::query(
            r#"INSERT INTO daily_market_forward_return (date, market_id, n5, sum5, win5, n20, n60, n120, n240)
VALUES ($1, 0, $2, 3.0, 2.0, 0, 0, 0, 0)"#,
        )
        .bind(date)
        .bind(n5)
        .execute(&mut *tx)
        .await
        .expect("寫入彙總應成功");
    }

    let drift: Vec<NaiveDate> = sqlx::query_scalar(MARKET_FORWARD_RETURN_DRIFT_SQL)
        .bind(day(1))
        .fetch_all(&mut *tx)
        .await
        .expect("查詢應成功");
    tx.rollback().await.expect("rollback 應成功");

    assert_eq!(drift, [day(2), day(3), day(4)]);
}
