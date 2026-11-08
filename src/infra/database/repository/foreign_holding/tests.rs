//! 外資持股歷史與趨勢的 PostgreSQL 倉儲實作的測試。

use rust_decimal_macros::dec;

use super::*;

const FAKE_SYMBOL: &str = "79979QF";

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
}

async fn cleanup() {
    let pool = database::get_connection();
    let _ = sqlx::query("DELETE FROM qfii_history WHERE stock_symbol = $1")
        .bind(FAKE_SYMBOL)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM qfii_trend WHERE stock_symbol = $1")
        .bind(FAKE_SYMBOL)
        .execute(pool)
        .await;
}

fn holding(day: NaiveDate, held: i64, pct: Decimal) -> ForeignHolding {
    ForeignHolding {
        stock_symbol: FAKE_SYMBOL.to_string(),
        date: day,
        issued_share: 1_000_000,
        shares_held: held,
        share_holding_percentage: pct,
    }
}

/// 21 個交易日的假資料：前 18 天持平、最後 3 天連續增持。
/// 驗證 5 日／20 日變化與連續天數，以及重送同一天不重複計數。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_save_daily_and_rebuild_trend() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_save_daily_and_rebuild_trend：無資料庫連接");
        return;
    }
    cleanup().await;
    let repo = PgForeignHoldingRepository::new();

    let start = date(1990, 1, 1);
    let mut rows = Vec::new();
    for i in 0..21_i64 {
        let day = start + chrono::Days::new(i as u64);
        let (held, pct) = match i {
            0..=17 => (100_000, dec!(10)),
            18 => (110_000, dec!(11)),
            19 => (120_000, dec!(12)),
            _ => (130_000, dec!(13)),
        };
        rows.push(holding(day, held, pct));
    }
    assert_eq!(repo.save_daily(&rows).await.expect("save"), 21);
    // 內容相同的重送不算變更
    assert_eq!(repo.save_daily(&rows).await.expect("save again"), 0);

    let as_of = start + chrono::Days::new(20);
    assert!(repo.rebuild_trends_as_of(as_of).await.expect("rebuild") >= 1);

    let row = sqlx::query(
        "SELECT change_5d, change_20d, streak, share_holding_percentage FROM qfii_trend WHERE stock_symbol = $1",
    )
    .bind(FAKE_SYMBOL)
    .fetch_one(database::get_connection())
    .await
    .expect("trend row");
    let change_5d: Option<Decimal> = row.get("change_5d");
    let change_20d: Option<Decimal> = row.get("change_20d");
    let streak: i32 = row.get("streak");
    assert_eq!(change_5d, Some(dec!(3)));
    assert_eq!(change_20d, Some(dec!(3)));
    assert_eq!(streak, 3);

    cleanup().await;
}
