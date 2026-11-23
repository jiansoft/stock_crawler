//! 持股三大法人與融資融券通知的測試。
//!
//! 子模組：`reasons`（通知理由判斷）、`message`（訊息格式）、`report`（挑出要通知的持股）、
//! `streak_store`（Redis 存取）、`sources`（來源抓取與排程入口）；這裡放共用的日期、法人買賣與報告建構函式。

mod message;
mod reasons;
mod report;
mod sources;
mod streak_store;

use super::*;
use crate::core::util::datetime::ymd;
use jiff::civil::Date;

pub(super) fn day(d: u32) -> Date {
    ymd(2026, 10, d).unwrap()
}

pub(super) fn flow(foreign: i64, trust: i64, dealer: i64) -> InstitutionalFlow {
    InstitutionalFlow {
        foreign,
        trust,
        dealer,
    }
}

pub(super) fn report(
    flow: Option<InstitutionalFlow>,
    margin: Option<MarginBalance>,
    volume: Option<i64>,
    streak: Streak,
) -> ChipReport {
    ChipReport {
        stock_symbol: "2330".to_string(),
        flow,
        margin,
        volume,
        streak,
    }
}

/// 收件者範圍的籌碼通知以資料庫的持股為準：持有 2330 的會員收到，沒有持股的會員不產生訊息。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn view_message_uses_holdings_in_scope() {
    use crate::domain::portfolio::view::HoldingView;
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 view_message_uses_holdings_in_scope：無資料庫連接");
        return;
    }
    const MEMBER: i64 = 979_905;
    let cleanup = || async {
        sqlx::query("DELETE FROM stock_ownership_details WHERE member_id = $1")
            .bind(MEMBER)
            .execute(pool)
            .await
            .expect("清除測試持股");
    };
    cleanup().await;
    sqlx::query(
        "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, created_time, date) \
         VALUES ($1, '2330', 1000, '1999-03-10 00:00:00+08', '1999-03-10')",
    )
    .bind(MEMBER)
    .execute(pool)
    .await
    .expect("插入測試持股");
    let reports = vec![report(
        Some(flow(9_770_520, 589_108, 491_655)),
        None,
        Some(87_000_000),
        Streak {
            date: day(6),
            foreign: 3,
            trust: 1,
        },
    )];
    let held = view_message(day(6), &reports, HoldingView::member(1, MEMBER)).await;
    let empty = view_message(day(6), &reports, HoldingView::member(1, MEMBER + 1)).await;
    cleanup().await;
    assert!(
        held.expect("查詢持股").is_some(),
        "持有 2330 應收到籌碼通知"
    );
    assert_eq!(empty.expect("查詢持股"), None);
}
