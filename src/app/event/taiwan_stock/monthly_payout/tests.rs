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
        forecast: None,
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
        forecast: None,
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
    notify(&repo, None, today).await.expect("通知");

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

/// 有快照誤差時在年累計之後附一行：已配對的預估、實際與差額，以及漏領筆數。
#[test]
fn build_message_appends_snapshot_accuracy() {
    let summary = MonthlySummary {
        from: date(2026, 12, 1),
        to: date(2026, 12, 31),
        payouts: vec![payout(1, "0056", 11, dec!(2872.4))],
        last_year_cash: Decimal::ZERO,
        year_to_date_cash: dec!(2872.4),
        forecast: Some(SnapshotAccuracy {
            snapshot_date: date(2026, 12, 1),
            month: MonthAccuracy {
                matched_count: 2,
                matched_forecast: dec!(6400),
                matched_actual: dec!(6772.4),
                missed_count: 1,
                missed_forecast: dec!(500),
            },
        }),
    };

    let message = build_message(&summary).expect("上月有入帳");

    let line = r"12\-01 快照預估誤差：比對 2 筆，預估 6,400 元、實際 6,772\.4 元（\+372\.4 元，\+5\.5%）；預估有入帳但沒領到 1 筆（預估 500 元）";
    assert!(message.contains(line), "缺少 {line}：\n{message}");
    let accuracy_at = message.find("快照預估誤差").expect("有誤差行");
    let note_at = message.find("金額未扣").expect("有備註");
    assert!(accuracy_at < note_at, "誤差行放在備註之前：\n{message}");
}

/// 只有漏領、或實際比預估少時的文字（負號不重複加 +）。
#[test]
fn forecast_text_handles_missed_only_and_shortfall() {
    let snapshot_date = date(2026, 11, 1);
    let missed_only = SnapshotAccuracy {
        snapshot_date,
        month: MonthAccuracy {
            missed_count: 2,
            missed_forecast: dec!(1200),
            ..MonthAccuracy::default()
        },
    };
    assert_eq!(
        forecast_text(&missed_only),
        "11-01 快照預估誤差：預估有入帳但沒領到 2 筆（預估 1,200 元）"
    );

    let shortfall = SnapshotAccuracy {
        snapshot_date,
        month: MonthAccuracy {
            matched_count: 1,
            matched_forecast: dec!(1000),
            matched_actual: dec!(800),
            ..MonthAccuracy::default()
        },
    };
    assert_eq!(
        forecast_text(&shortfall),
        "11-01 快照預估誤差：比對 1 筆，預估 1,000 元、實際 800 元（-200 元，-25%）"
    );
}

/// 讀取上個月底以前最近一次快照：12-01 的通知用 11-01 快照，不用 12-01 當天的新快照；
/// 快照的 11 月預估沒有入帳時算漏領（臨時庫，固定 2019 年、假代號自我清理）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn load_snapshot_accuracy_uses_latest_snapshot_before_month_end() {
    dotenvy::dotenv().ok();
    if crate::infra::database::ping().await.is_err() {
        println!("跳過 load_snapshot_accuracy_uses_latest_snapshot_before_month_end：無資料庫連接");
        return;
    }
    let pool = crate::infra::database::get_connection();
    let cleanup = || async {
        let _ = sqlx::query("DELETE FROM dividend_forecast_snapshot WHERE stock_symbol = '79979N'")
            .execute(pool)
            .await;
    };
    cleanup().await;
    // 11-01 快照：11-20 預估 500 元；12-01 快照：12-20 預估 900 元（不該被拿來比對 11 月）。
    sqlx::query(
        r"INSERT INTO dividend_forecast_snapshot
              (snapshot_date, member_id, stock_symbol, ex_date, pay_date, shares, cash_per_share, amount, basis)
          VALUES ('2019-11-01', 79979, '79979N', '2019-11-05', '2019-11-20', 1000, 0.5, 500, 'announced'),
                 ('2019-12-01', 79979, '79979N', '2019-12-05', '2019-12-20', 1000, 0.9, 900, 'announced')",
    )
    .execute(pool)
    .await
    .expect("種入快照");

    let result = load_snapshot_accuracy(date(2019, 12, 1)).await;
    let before_any = load_snapshot_accuracy(date(2019, 11, 1)).await;
    cleanup().await;

    let accuracy = result.expect("讀取快照誤差").expect("11 月有預估");
    assert_eq!(accuracy.snapshot_date, date(2019, 11, 1));
    assert_eq!(
        accuracy.month,
        MonthAccuracy {
            missed_count: 1,
            missed_forecast: dec!(500),
            ..MonthAccuracy::default()
        }
    );
    assert!(
        matches!(before_any, Ok(None)),
        "10 月底以前沒有快照時不附誤差：{before_any:?}"
    );
}
