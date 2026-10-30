//! 外資持股回補的單元測試。

use rust_decimal_macros::dec;

use crate::infra::cache::SHARE;

use super::*;

#[test]
fn to_history_stamps_every_row_with_the_data_date() {
    let rows = vec![QfiiDto {
        stock_symbol: "2330".to_string(),
        issued_share: 25_932_370_067,
        shares_held: 17_951_023_616,
        share_holding_percentage: dec!(69.22),
    }];
    let date = NaiveDate::from_ymd_opt(2026, 8, 28).unwrap();

    let history = to_history(&rows, date);

    assert_eq!(
        history,
        vec![ForeignHolding {
            stock_symbol: "2330".to_string(),
            date,
            issued_share: 25_932_370_067,
            shares_held: 17_951_023_616,
            share_holding_percentage: dec!(69.22),
        }]
    );
}

fn dto(symbol: &str) -> QfiiDto {
    QfiiDto {
        stock_symbol: symbol.to_string(),
        issued_share: 1_000,
        shares_held: 300,
        share_holding_percentage: dec!(30),
    }
}

fn day(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).unwrap()
}

/// 週末略過；起訖同一天只有那一天。
#[test]
fn weekdays_between_skips_weekends() {
    // 2026-10-09（五）～10-13（二）：跳過 10-10、10-11。
    assert_eq!(
        weekdays_between(day(10, 9), day(10, 13)).unwrap(),
        vec![day(10, 9), day(10, 12), day(10, 13)]
    );
    assert_eq!(
        weekdays_between(day(10, 12), day(10, 12)).unwrap(),
        vec![day(10, 12)]
    );
    assert!(
        weekdays_between(day(10, 10), day(10, 11))
            .unwrap()
            .is_empty()
    );
}

/// 上櫃回應日期與查詢日相符才收；不符（休市回前一日）只留上市。
#[test]
fn day_history_keeps_otc_only_for_matching_date() {
    use twse::qualified_foreign_institutional_investor::over_the_counter::OtcQfiiSnapshot;
    let otc = OtcQfiiSnapshot {
        date: day(10, 12),
        rows: vec![dto("6488")],
    };
    let merged = day_history(&[dto("2330")], &otc, day(10, 12));
    assert_eq!(merged.len(), 2);
    assert!(merged.iter().all(|holding| holding.date == day(10, 12)));
    let stale = day_history(&[dto("2330")], &otc, day(10, 13));
    assert_eq!(stale.len(), 1);
}

/// 基準日：上市有資料取較晚者；上市休市用上櫃資料日。
#[test]
fn trend_as_of_picks_latest_available_date() {
    assert_eq!(trend_as_of(false, day(10, 12), day(10, 9)), day(10, 12));
    assert_eq!(trend_as_of(false, day(10, 12), day(10, 13)), day(10, 13));
    assert_eq!(trend_as_of(true, day(10, 12), day(10, 9)), day(10, 9));
}

/// 三個欄位都相同才不需要更新。
#[test]
fn qfii_changed_compares_all_fields() {
    let cmd = UpdateQfiiCommand {
        symbol: "2330".to_string(),
        shares_held: 300,
        share_holding_percentage: dec!(30),
        issued_share: 1_000,
    };
    assert!(!qfii_changed((1_000, 300, dec!(30)), &cmd));
    assert!(qfii_changed((1_000, 301, dec!(30)), &cmd));
    assert!(qfii_changed((1_000, 300, dec!(30.01)), &cmd));
    assert!(qfii_changed((999, 300, dec!(30)), &cmd));
}

/// 逐日迴圈：只抓平日、有資料才寫、每個平日後停頓、回傳最後一個有資料的交易日。
#[tokio::test]
async fn run_history_saves_trading_days_only() {
    let fetched = std::sync::Mutex::new(Vec::new());
    let pauses = std::sync::Mutex::new(0);
    let (summary, last) = run_history(
        day(10, 8),
        day(10, 12),
        |date| {
            fetched.lock().unwrap().push(date);
            async move {
                // 10-09 國慶補假休市：兩邊都沒有資料。
                Ok(if date == day(10, 9) {
                    Vec::new()
                } else {
                    to_history(&[dto("2330"), dto("6488")], date)
                })
            }
        },
        |history| async move { Ok(history.len() as u64) },
        || {
            *pauses.lock().unwrap() += 1;
            async {}
        },
    )
    .await
    .unwrap();
    assert_eq!(
        *fetched.lock().unwrap(),
        vec![day(10, 8), day(10, 9), day(10, 12)]
    );
    assert_eq!(*pauses.lock().unwrap(), 3);
    assert_eq!((summary.trading_days, summary.saved_rows), (2, 4));
    assert_eq!(last, Some(day(10, 12)));

    let failed = run_history(
        day(10, 12),
        day(10, 12),
        |_| async { Err(anyhow!("封鎖")) },
        |_| async { Ok(0) },
        || async {},
    )
    .await;
    assert!(failed.is_err());
}

#[tokio::test]
async fn backfill_history_rejects_reversed_range() {
    let start = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
    assert!(backfill_history(start, end).await.is_err());
}

/// 驗證外資持股回補流程。
#[tokio::test]
#[ignore]
async fn test_execute() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 execute");

    match execute().await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("Failed to execute because {:?}", why);
        }
    }

    tracing::debug!("結束 execute");
}
