//! `MoneyFlowRecalculated` 事件處理：重新計算並發送 Telegram 市值變化通知的測試。

use super::*;
use jiff::civil::date;

#[test]
fn test_build_money_change_message_includes_hugo() {
    use crate::domain::money_flow::entity::MoneyFlowMemberWithPreviousDay;
    use jiff::civil::Date;
    use rust_decimal_macros::dec;

    let date = Date::strptime("%Y-%m-%d", "2026-04-02").unwrap();
    let previous_date = Date::strptime("%Y-%m-%d", "2026-04-01").unwrap();
    let rows = vec![
        MoneyFlowMemberWithPreviousDay {
            household_id: 1,
            date,
            previous_date: Some(previous_date),
            member_id: 0,
            market_value: dec!(4273187.20),
            previous_market_value: dec!(4053774.55),
        },
        MoneyFlowMemberWithPreviousDay {
            household_id: 1,
            date,
            previous_date: Some(previous_date),
            member_id: 1,
            market_value: dec!(2195395.10),
            previous_market_value: dec!(2207807.70),
        },
        MoneyFlowMemberWithPreviousDay {
            household_id: 1,
            date,
            previous_date: Some(previous_date),
            member_id: 2,
            market_value: dec!(1500000.00),
            previous_market_value: dec!(1400000.00),
        },
        MoneyFlowMemberWithPreviousDay {
            household_id: 1,
            date,
            previous_date: Some(previous_date),
            member_id: 3,
            market_value: dec!(577792.10),
            previous_market_value: dec!(445966.85),
        },
    ];

    let msg = EventDispatcher::build_money_change_message(
        &rows,
        &crate::app::event::taiwan_stock::test_member_names(),
    )
    .expect("message should be built");

    assert!(msg.contains("合計"));
    assert!(msg.contains("Eddie"));
    assert!(msg.contains("Unice"));
    assert!(msg.contains("Hugo"));
    assert!(msg.contains("4,273,187\\.20"));
    assert!(msg.contains("577,792\\.10"));
    assert!(msg.contains("\\-12,412\\.60"));
}

/// 只含本人時只有本人一列（沒有合計）；含家庭成員時為家庭成員與合計；其他家庭不列預設家庭的合計。
#[test]
fn rows_in_view_follows_recipient_scope() {
    let row = |member_id: i64| MoneyFlowMemberWithPreviousDay {
        household_id: 1,
        date: date(2026, 11, 17),
        previous_date: None,
        member_id,
        market_value: rust_decimal::Decimal::ONE,
        previous_market_value: rust_decimal::Decimal::ONE,
    };
    let mut other_total = row(0);
    other_total.household_id = 2;
    let mut other_member = row(9);
    other_member.household_id = 2;
    let rows = vec![row(0), row(1), row(2), other_total, other_member];
    let ids = |rows: Vec<MoneyFlowMemberWithPreviousDay>| -> Vec<i64> {
        rows.into_iter().map(|row| row.member_id).collect()
    };
    assert_eq!(
        ids(rows_in_view(&rows, &HoldingView::member(1, 2), &[2])),
        [2]
    );
    assert_eq!(
        ids(rows_in_view(&rows, &HoldingView::household(1), &[1, 2])),
        [0, 1, 2]
    );
    assert_eq!(
        ids(rows_in_view(&rows, &HoldingView::household(2), &[9])),
        [0, 9]
    );
}

/// 收件者範圍的訊息：預設家庭查得到會員名稱；範圍內沒有市值列時不產生訊息。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn view_message_reads_names_in_scope() {
    dotenvy::dotenv().ok();
    if sqlx::query("SELECT 1")
        .execute(crate::infra::database::get_connection())
        .await
        .is_err()
    {
        println!("跳過 view_message_reads_names_in_scope：無資料庫連接");
        return;
    }
    let row = MoneyFlowMemberWithPreviousDay {
        household_id: 979_902,
        date: date(1999, 3, 10),
        previous_date: None,
        member_id: 0,
        market_value: rust_decimal::Decimal::TEN,
        previous_market_value: rust_decimal::Decimal::ONE,
    };
    let other_household =
        view_message(std::slice::from_ref(&row), HoldingView::default_household())
            .await
            .expect("讀取會員名稱");
    assert_eq!(
        other_household, None,
        "其他家庭的合計不可出現在預設家庭的訊息"
    );
    let own = view_message(&[row], HoldingView::household(979_902))
        .await
        .expect("讀取會員名稱")
        .expect("範圍內有合計列時要有訊息");
    assert!(own.contains("合計"));
}
