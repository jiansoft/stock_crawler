//! 次日開盤停損的真實 router、查詢與成交價格整合驗證。

use super::*;

/// 收盤觸發後次日開盤賣出、停牌順延、已到期不重複成交，並可比較成交假設。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要臨時 PostgreSQL")]
async fn screen_backtest_next_open_stops_execute_at_actual_open() {
    let Some(pool) = seeded("screen_backtest_next_open_stops").await else {
        return;
    };
    sqlx::query("INSERT INTO daily_forward_return(date,stock_symbol,return_5d) VALUES ('1993-03-02','79979BA',10)")
        .execute(pool).await.unwrap();
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date",stock_symbol,"OpeningPrice","ClosingPrice") VALUES
        ('1993-03-02','79979BA',100,110),('1993-03-03','79979BA',95,88),
        ('1993-03-04','79979BA',85,90),('1993-03-09','79979BA',133.1,121)"#,
    )
    .execute(pool)
    .await
    .unwrap();
    let base = "limit=1&min_rs_score=85&execution_price=next_open&stop_loss=10";
    let (status, json) = query(base).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["periods"][0]["avg_return"], -15.0, "{json}");
    assert_eq!(json["periods"][0]["stopped"], 1);
    assert_eq!(json["periods"][0]["return_after_costs"], -15.52);
    assert!(json["summary"]["daily_max_drawdown_after_costs"].is_number());
    let (_, trailing) =
        query("limit=1&min_rs_score=85&execution_price=next_open&trailing_stop=10").await;
    assert_eq!(trailing["periods"][0]["avg_return"], -15.0);
    // 缺開盤即不可成交，順延到下一個有效開盤；不是沿用當天收盤賣出。
    sqlx::query(r#"UPDATE "DailyQuotes" SET "OpeningPrice"=0 WHERE stock_symbol='79979BA' AND "Date"='1993-03-04'"#).execute(pool).await.unwrap();
    sqlx::query(r#"INSERT INTO "DailyQuotes" ("Date",stock_symbol,"OpeningPrice","ClosingPrice") VALUES ('1993-03-05','79979BA',70,80)"#).execute(pool).await.unwrap();
    let (_, rolled) = query(base).await;
    assert_eq!(rolled["periods"][0]["avg_return"], -30.0);
    assert_eq!(rolled["periods"][0]["stopped"], 1);
    sqlx::query(
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol='79979BA' AND "Date"='1993-03-05'"#,
    )
    .execute(pool)
    .await
    .unwrap();
    let (_, at_end) = query(base).await;
    assert_eq!(at_end["periods"][0]["avg_return"], 33.1);
    assert_eq!(at_end["periods"][0]["stopped"], 0);
    let (status,comparison)=get("/api/v1/stocks/screen/backtest/compare?from=1993-03-01&to=1993-03-15&hold_days=5&limit=1&stop_loss=10&scenario=execution_price%3Dclose&scenario=execution_price%3Dnext_open").await;
    assert_eq!(status, StatusCode::OK, "{comparison}");
    assert_eq!(comparison["scenarios"].as_array().unwrap().len(), 2);
    cleanup(pool).await;
}
