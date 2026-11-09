//! 每日風險指標重算的整合測試（臨時庫、假代號、固定 1985 年資料）。

use chrono::{Days, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::*;
use crate::infra::database;

const SYMBOLS: [&str; 3] = ["79979RA", "79979RB", "79979RC"];

fn day(n: u64) -> NaiveDate {
    NaiveDate::from_ymd_opt(1985, 1, 1)
        .and_then(|first| first.checked_add_days(Days::new(n)))
        .expect("固定日期合法")
}

async fn cleanup(pool: &sqlx::PgPool) {
    for sql in [
        "DELETE FROM daily_risk_metric WHERE stock_symbol = ANY($1)",
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(SYMBOLS).execute(pool).await;
    }
    let _ =
        sqlx::query(r#"DELETE FROM "index" WHERE category = 'TAIEX' AND date BETWEEN $1 AND $2"#)
            .bind(day(0))
            .bind(day(399))
            .execute(pool)
            .await;
}

/// 1985 年起連續 250 天：加權指數日報酬交替 +1%／−1%（漲跌點 ±1、指數 101／99）；RA 每天漲跌幅是大盤的 2 倍
/// （±2%）、RB 每天 +0.5%、RC 只有後 150 天。只重算最後 5 天：RA、RB 各 5 列（RC 樣本不足 200 筆不寫入），
/// 窗口外的舊列不動；RA 的 beta 為 2、年化波動度為 ±2% 交替的樣本標準差 × √240，RB 波動度與 beta 都是 0。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn calculate_risk_metrics_between_writes_rolling_volatility_and_beta() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!(
            "跳過 calculate_risk_metrics_between_writes_rolling_volatility_and_beta：無資料庫連接"
        );
        return;
    }
    let pool = database::get_connection();
    cleanup(pool).await;
    for symbol in SYMBOLS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '風險指標', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
    }
    sqlx::query(
        r#"INSERT INTO "index" (category, date, change, index)
           SELECT 'TAIEX', $1::date + n, CASE WHEN n % 2 = 0 THEN 1 ELSE -1 END, CASE WHEN n % 2 = 0 THEN 101 ELSE 99 END
           FROM generate_series(0, 249) AS n"#,
    )
    .bind(day(0))
    .execute(pool)
    .await
    .expect("種入加權指數");
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ChangeRange")
           SELECT $1::date + n, '79979RA', CASE WHEN n % 2 = 0 THEN 2 ELSE -2 END FROM generate_series(0, 249) AS n
           UNION ALL SELECT $1::date + n, '79979RB', 0.5 FROM generate_series(0, 249) AS n
           UNION ALL SELECT $1::date + n, '79979RC', 1 FROM generate_series(100, 249) AS n"#,
    )
    .bind(day(0))
    .execute(pool)
    .await
    .expect("種入日線");
    // 窗口外（第 0 天）先放一列舊資料，確認只替換指定日期。
    sqlx::query(
        "INSERT INTO daily_risk_metric (date, stock_symbol, volatility_percent, beta, samples)
         VALUES ($1, '79979RA', 99, 9, 200)",
    )
    .bind(day(0))
    .execute(pool)
    .await
    .expect("種入舊資料");

    let written = calculate_risk_metrics_between(day(245), day(249))
        .await
        .expect("重算風險指標");
    let rows: Vec<(NaiveDate, String, Decimal, Option<Decimal>, i16)> = sqlx::query_as(
        "SELECT date, stock_symbol, volatility_percent, beta, samples FROM daily_risk_metric
         WHERE stock_symbol = ANY($1) ORDER BY stock_symbol, date",
    )
    .bind(SYMBOLS)
    .fetch_all(pool)
    .await
    .expect("讀取風險指標");
    let same_day = calculate_risk_metrics(day(249)).await.expect("重算單日");
    cleanup(pool).await;

    assert_eq!(written, 10);
    assert_eq!(same_day, 2);
    // ±2% 交替 240 筆：樣本標準差 = 0.02 × √(240 / 239)，年化再乘 √240。
    let expected = Decimal::try_from(0.02 * (240.0_f64 / 239.0).sqrt() * 240.0_f64.sqrt() * 100.0)
        .expect("有限數值")
        .round_dp(2);
    let ra: Vec<_> = rows.iter().filter(|row| row.1 == "79979RA").collect();
    assert_eq!(ra.len(), 6, "{rows:?}");
    assert_eq!((ra[0].0, ra[0].2), (day(0), dec!(99)), "窗口外的舊列不動");
    for row in &ra[1..] {
        assert_eq!(
            (row.2, row.3, row.4),
            (expected, Some(dec!(2.000)), 240),
            "{row:?}"
        );
    }
    let rb: Vec<_> = rows.iter().filter(|row| row.1 == "79979RB").collect();
    assert_eq!(rb.len(), 5, "{rows:?}");
    assert!(
        rb.iter()
            .all(|row| row.2 == dec!(0) && row.3 == Some(dec!(0.000)) && row.4 == 240),
        "{rb:?}"
    );
    assert!(rows.iter().all(|row| row.1 != "79979RC"), "樣本不足不寫入");
}
