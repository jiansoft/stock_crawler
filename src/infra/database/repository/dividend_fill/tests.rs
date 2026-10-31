//! 全市場填息樣本查詢的整合測試：事件規則、依配股調整的目標價與無效日期。

use rust_decimal_macros::dec;

use super::*;
use crate::domain::dividend::fill;

const SYMBOL: &str = "7998";

async fn cleanup(pool: &sqlx::PgPool) {
    for sql in [
        "DELETE FROM dividend WHERE security_code = $1",
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
    ] {
        let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
    }
}

/// 2001-03 的三次事件：03-05 純現金（彙總列不重複取）、03-20 現金＋配股（目標 66 ÷ 1.1 = 60）、
/// 03-12 只有配股（沒有現金股利不列入）；另有 02-30 的無效日期不可讓整批查詢失敗。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn market_fill_samples_follow_event_rules() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 market_fill_samples_follow_event_rules：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    cleanup(pool).await;
    for sql in [
        r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2")
           VALUES ($1, 2001, 2000, 'Q4', 2, 0, 2, '2001-03-05', '-'),
                  ($1, 2001, 2000, '', 10, 0, 10, '2001-03-05', '-'),
                  ($1, 2001, 2001, 'Q1', 0, 1, 1, '-', '2001-03-12'),
                  ($1, 2001, 2001, 'Q2', 5, 1, 6, '2001-03-20', '2001-03-20'),
                  ($1, 2001, 2001, 'Q3', 1, 0, 1, '2001-02-30', '-')"#,
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, year, month, day, "OpeningPrice", "HighestPrice", "LowestPrice", "ClosingPrice")
           VALUES ('2001-03-02', $1, 2001, 3, 2, 100, 100, 100, 100),
                  ('2001-03-05', $1, 2001, 3, 5, 97, 97, 97, 97),
                  ('2001-03-06', $1, 2001, 3, 6, 99, 99, 99, 99),
                  ('2001-03-07', $1, 2001, 3, 7, 100, 100, 100, 100),
                  ('2001-03-19', $1, 2001, 3, 19, 66, 66, 66, 66),
                  ('2001-03-20', $1, 2001, 3, 20, 58, 58, 58, 58),
                  ('2001-03-21', $1, 2001, 3, 21, 61, 61, 61, 61)"#,
    ] {
        sqlx::query(sql)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .expect("寫入測試資料");
    }

    let day = |m: u32, d: u32| NaiveDate::from_ymd_opt(2001, m, d).expect("日期應合法");
    let samples = PgDividendFillRepository::new()
        .fetch_market_fill_samples(day(2, 1), day(3, 31))
        .await;
    cleanup(pool).await;
    let mut samples = samples.expect("查詢應成功");
    samples.sort_by_key(|s| s.bucket);

    assert_eq!(fill::fill_target(dec!(66), dec!(1)), dec!(60));
    assert_eq!(
        samples,
        [
            FillSample {
                bucket: YieldBucket::Below3,
                trading_days_to_fill: Some(3),
                trading_days_observed: 6,
            },
            FillSample {
                bucket: YieldBucket::AtLeast7,
                trading_days_to_fill: Some(2),
                trading_days_observed: 2,
            },
        ],
        "03-05 現金 2 元（2%）第 3 天回到 100；03-20 現金 5 元（7.6%）第 2 天站上 60"
    );
}
