//! 每日股價淨值比分位重算的整合測試（臨時庫、假代號、固定 1983 年起的資料）。

use chrono::{Days, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::*;
use crate::infra::database;

const SYMBOLS: [&str; 4] = ["79979PA", "79979PB", "79979PC", "79979PD"];

fn day(n: u64) -> NaiveDate {
    NaiveDate::from_ymd_opt(1983, 1, 1)
        .and_then(|first| first.checked_add_days(Days::new(n)))
        .expect("固定日期合法")
}

async fn cleanup(pool: &sqlx::PgPool) {
    for sql in [
        "DELETE FROM daily_pb_percentile WHERE stock_symbol = ANY($1)",
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(SYMBOLS).execute(pool).await;
    }
}

/// 1983 年起連續 600 天（都在 5 年內）：PA 淨值比逐日上升（1 + n ÷ 100）、PB 逐日下降（10 − n ÷ 100）、
/// PC 只有後 300 天（樣本不足 480 筆不寫入）、PD 在 1～10 循環（1 + n mod 10），第 600 天 PD 淨值比為 0（不算樣本）。
/// 重算第 595～600 天：PA 都在 5 年最高（100）、PB 都在最低（0）；PD 最後一天是 10，比它低的 540 天 ÷ 599。
/// 窗口外的舊列不動。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn calculate_pb_percentiles_between_ranks_against_five_years() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 calculate_pb_percentiles_between_ranks_against_five_years：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    cleanup(pool).await;
    for symbol in SYMBOLS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '淨值比分位', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
    }
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "price-to-book_ratio")
           SELECT $1::date + n, '79979PA', 1 + n / 100.0 FROM generate_series(0, 599) AS n
           UNION ALL SELECT $1::date + n, '79979PB', 10 - n / 100.0 FROM generate_series(0, 599) AS n
           UNION ALL SELECT $1::date + n, '79979PC', 2 FROM generate_series(300, 599) AS n
           UNION ALL SELECT $1::date + n, '79979PD', 1 + n % 10 FROM generate_series(0, 599) AS n
           UNION ALL SELECT $1::date + 600, '79979PD', 0"#,
    )
    .bind(day(0))
    .execute(pool)
    .await
    .expect("種入日線");
    // 窗口外（第 0 天）先放一列舊資料，確認只替換指定日期。
    sqlx::query(
        "INSERT INTO daily_pb_percentile (date, stock_symbol, pb, pb_percentile, samples)
         VALUES ($1, '79979PA', 1, 55, 480)",
    )
    .bind(day(0))
    .execute(pool)
    .await
    .expect("種入舊資料");

    let written = calculate_pb_percentiles_between(day(595), day(600))
        .await
        .expect("重算淨值比分位");
    let rows: Vec<(NaiveDate, String, Decimal, Decimal, i16)> = sqlx::query_as(
        "SELECT date, stock_symbol, pb, pb_percentile, samples FROM daily_pb_percentile
         WHERE stock_symbol = ANY($1) ORDER BY stock_symbol, date",
    )
    .bind(SYMBOLS)
    .fetch_all(pool)
    .await
    .expect("讀取淨值比分位");
    let same_day = calculate_pb_percentiles(day(599)).await.expect("重算單日");
    cleanup(pool).await;

    // 第 600 天只有 PD 一列且淨值比為 0，不算樣本也不寫入。
    assert_eq!(written, 15);
    assert_eq!(same_day, 3);
    let of = |symbol: &str| -> Vec<_> { rows.iter().filter(|row| row.1 == symbol).collect() };
    let pa = of("79979PA");
    assert_eq!(pa.len(), 6, "{rows:?}");
    assert_eq!((pa[0].0, pa[0].3), (day(0), dec!(55)), "窗口外的舊列不動");
    assert!(pa[1..].iter().all(|row| row.3 == dec!(100)), "{pa:?}");
    assert_eq!((pa[5].0, pa[5].4), (day(599), 600));
    let pb = of("79979PB");
    assert_eq!(pb.len(), 5, "{rows:?}");
    assert!(pb.iter().all(|row| row.3 == dec!(0)), "{pb:?}");
    assert_eq!((pb[4].2, pb[4].4), (dec!(4.01), 600));
    assert!(of("79979PC").is_empty(), "樣本不足不寫入");
    let pd = of("79979PD");
    assert_eq!(pd.len(), 5, "{rows:?}");
    assert_eq!(
        (pd[4].0, pd[4].2, pd[4].3),
        (day(599), dec!(10), dec!(90.15))
    );
}
