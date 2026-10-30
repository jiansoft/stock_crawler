//! 全市場區間突破通知連資料庫的整合測試（排行、排程入口、前幾日突破後續）。

use super::*;

/// 連資料庫：依成交值排序、取前 N 檔、總檔數為當天全部；其他來源、其他種類（假突破）與其他日期不列入。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn fetch_ranked_signals_orders_by_trade_value() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 fetch_ranked_signals_orders_by_trade_value：無資料庫連接");
        return;
    }
    let day = NaiveDate::from_ymd_opt(1990, 3, 1).expect("有效日期");
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979B_'"#,
            "DELETE FROM daily_technical_signal WHERE stock_symbol LIKE '79979B_'",
            "DELETE FROM stocks WHERE stock_symbol LIKE '79979B_'",
        ] {
            sqlx::query(sql).execute(pool).await.expect("清理測試資料");
        }
    };
    cleanup().await;
    // 79979BD 成交值最大但只有假突破，不列入突破排行。
    for (symbol, trade_value, kind) in [
        ("79979BA", 100, "breakout"),
        ("79979BB", 300, "breakout"),
        ("79979BC", 200, "breakout"),
        ("79979BD", 900, "failed"),
    ] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, $1, $1, 2, 24)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("寫入測試股票");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "TradeValue") VALUES ($1, $2, 10, $3)"#,
        )
        .bind(day)
        .bind(symbol)
        .bind(trade_value)
        .execute(pool)
        .await
        .expect("寫入測試日線");
        sqlx::query(
            "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
             VALUES ($1, $2, 'range_breakout', $3, $2), ($1, $2, 'kd', 'overbought', 'kd')",
        )
        .bind(day)
        .bind(symbol)
        .bind(kind)
        .execute(pool)
        .await
        .expect("寫入測試訊號");
    }

    let (signals, total) = PgQuoteRepository::new()
        .fetch_ranked_signals(day, RANGE_BREAKOUT_INDICATOR, SignalKind::Breakout, 2)
        .await
        .expect("查詢成功");

    assert_eq!(total, 3);
    let symbols: Vec<&str> = signals.iter().map(|s| s.stock_symbol.as_str()).collect();
    assert_eq!(symbols, ["79979BB", "79979BC"]);
    assert_eq!(signals[0].industry, "半導體業");
    assert_eq!(signals[0].trade_value, 300.0);
    cleanup().await;
}

/// 排程入口連資料庫走完：讀持股、查當天訊號；當天沒有訊號時不發送也不失敗。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn execute_runs_against_database() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 execute_runs_against_database：無資料庫連接");
        return;
    }

    execute().await.expect("沒有訊號時照常結束");
}

/// 連資料庫：前 N 個交易日（不含當天）的突破，同一檔只取最近一次，附之後是否跌回；更早與當天的不列入。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn fetch_breakout_follow_ups_reads_latest_breakout_and_failure() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 fetch_breakout_follow_ups_reads_latest_breakout_and_failure：無資料庫連接");
        return;
    }
    let day = |d: u32| NaiveDate::from_ymd_opt(1990, 5, d).expect("有效日期");
    let cleanup = || async {
        sqlx::query("DELETE FROM daily_technical_signal WHERE stock_symbol LIKE '79979F_'")
            .execute(pool)
            .await
            .expect("清理測試資料");
    };
    cleanup().await;
    // FA：第 1 天突破、第 3 天跌回；FB：第 2、4 天各突破一次，之後沒跌回；FC：太早；FD：查詢當天才突破。
    for (symbol, date, kind) in [
        ("79979FA", day(1), "breakout"),
        ("79979FA", day(3), "failed"),
        ("79979FB", day(2), "breakout"),
        ("79979FB", day(4), "breakout"),
        (
            "79979FC",
            NaiveDate::from_ymd_opt(1990, 4, 1).expect("有效日期"),
            "breakout",
        ),
        ("79979FD", day(5), "breakout"),
    ] {
        sqlx::query(
            "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
             VALUES ($1, $2, 'range_breakout', $3, $2)",
        )
        .bind(date)
        .bind(symbol)
        .bind(kind)
        .execute(pool)
        .await
        .expect("寫入測試訊號");
    }

    let follow_ups = PgQuoteRepository::new()
        .fetch_breakout_follow_ups(day(5), 5)
        .await;
    cleanup().await;

    let got: Vec<(String, NaiveDate, bool)> = follow_ups
        .expect("查詢成功")
        .into_iter()
        .filter(|follow_up| follow_up.stock_symbol.starts_with("79979F"))
        .map(|follow_up| {
            (
                follow_up.stock_symbol,
                follow_up.breakout_date,
                follow_up.failed,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("79979FA".to_owned(), day(1), true),
            ("79979FB".to_owned(), day(4), false),
        ]
    );
}
