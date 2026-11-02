//! 上月持股股利入帳彙總：月份推算、查詢期間、訊息組裝與去年同月比較。

use rust_decimal_macros::dec;

use super::*;
use crate::domain::portfolio::payout::PayoutAllocation;
use crate::domain::portfolio::test_double::FakePayoutRepository;

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
}

fn payout(member_id: i64, symbol: &str, day: u32, cash: Decimal) -> DividendPayout {
    DividendPayout {
        member_id,
        payable_date: date(2026, 11, day),
        stock_symbol: symbol.to_owned(),
        name: format!("名{symbol}"),
        quarter: "Q3".to_owned(),
        ex_date: Some(date(2026, 10, 20)),
        shares: 1000,
        cash_dividend: dec!(1),
        stock_dividend: Decimal::ZERO,
        cash_amount: cash,
        stock_shares: 0,
        estimated: false,
    }
}

/// 上個月的第一天與最後一天，跨年與二月都正確。
#[test]
fn previous_month_handles_year_and_february() {
    assert_eq!(
        previous_month(date(2026, 12, 1)),
        (date(2026, 11, 1), date(2026, 11, 30))
    );
    assert_eq!(
        previous_month(date(2027, 1, 1)),
        (date(2026, 12, 1), date(2026, 12, 31))
    );
    assert_eq!(
        previous_month(date(2028, 3, 15)),
        (date(2028, 2, 1), date(2028, 2, 29))
    );
}

/// 上月入帳合併會員逐筆列出（含配股），附各會員、合計、去年同月與年累計。
#[test]
fn build_message_lists_payouts_and_comparisons() {
    let mut with_stock = payout(2, "2330", 20, dec!(1500));
    with_stock.stock_shares = 25;
    let summary = MonthlySummary {
        from: date(2026, 11, 1),
        to: date(2026, 11, 30),
        payouts: vec![
            payout(1, "0056", 11, dec!(2872.4)),
            payout(1, "2330", 20, dec!(3900)),
            with_stock,
        ],
        last_year_cash: dec!(5000),
        year_to_date_cash: dec!(30123.4),
    };

    let message = build_message(&summary).expect("上月有入帳");

    for line in [
        r"2026\-11 持股股利入帳彙總",
        r"• 11\-11 0056 名0056 現金 2,872\.4 元",
        // 兩位會員同一筆股利合併成一行。
        r"• 11\-20 2330 名2330 現金 5,400 元、配股 25 股",
        r"各會員：Eddie 6,772\.4 元、Unice 1,500 元",
        r"合計現金 8,272\.4 元；去年同月 5,000 元（\+65\.4%）",
        r"2026 年累計現金 30,123\.4 元（截至 11\-30）",
        "金額未扣二代健保補充保費與匯費",
    ] {
        assert!(message.contains(line), "缺少 {line}：\n{message}");
    }
}

/// 上個月沒有入帳不發；去年同月沒有入帳、或比去年少時的文字。
#[test]
fn build_message_handles_empty_month_and_last_year_text() {
    let summary = MonthlySummary {
        from: date(2026, 11, 1),
        to: date(2026, 11, 30),
        payouts: Vec::new(),
        last_year_cash: dec!(5000),
        year_to_date_cash: Decimal::ZERO,
    };
    assert!(build_message(&summary).is_none());
    assert_eq!(last_year_text(dec!(100), Decimal::ZERO), "去年同月沒有入帳");
    assert_eq!(
        last_year_text(dec!(4000), dec!(5000)),
        "去年同月 5,000 元（-20%）"
    );
}

/// 依序查上個月、去年同月與今年累計，全部會員、以今天判斷推算。
#[tokio::test]
async fn notify_queries_previous_month_last_year_and_year_to_date() {
    let repo = FakePayoutRepository::with_allocations(vec![PayoutAllocation {
        member_id: 1,
        stock_symbol: "0056".to_owned(),
        name: "元大高股息".to_owned(),
        quarter: "Q3".to_owned(),
        cash_dividend: dec!(2),
        stock_dividend: Decimal::ZERO,
        cash_payable: Some(date(2026, 11, 11)),
        stock_payable: None,
        cash_ex: Some(date(2026, 10, 20)),
        stock_ex: None,
        shares: 1000,
        cash_amount: dec!(2000),
        stock_shares: Decimal::ZERO,
        estimated: false,
    }]);
    let today = date(2026, 12, 1);

    let summary = load_summary(&repo, today).await.expect("彙總");
    notify(&repo, today).await.expect("通知");

    assert_eq!(
        repo.queries()[..3],
        [
            (0, date(2026, 11, 1), date(2026, 11, 30), today),
            (0, date(2025, 11, 1), date(2025, 11, 30), today),
            (0, date(2026, 1, 1), date(2026, 11, 30), today),
        ]
    );
    assert_eq!(summary.payouts.len(), 1);
    assert_eq!(
        (summary.last_year_cash, summary.year_to_date_cash),
        (Decimal::ZERO, dec!(2000)),
        "替身每次都回同一筆，去年同月的期間外不算"
    );
}
