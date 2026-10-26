//! 每日收盤事件的測試：匯總流程、各市場成交家數短缺判斷、技術訊號／未來報酬缺漏的補算與報告。

use super::*;
use crate::infra::cache::SHARE;
use std::time::Duration;

/// 每日收盤事件主要匯總流程的整合測試。
#[tokio::test]
#[ignore]
async fn test_aggregate() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    tracing::debug!("開始 event::taiwan_stock::closing::aggregate");

    let current_date = NaiveDate::parse_from_str("2026-04-30", "%Y-%m-%d").unwrap();

    match aggregate(current_date).await {
        Ok(_) => {
            tracing::debug!(
                "{}",
                "event::taiwan_stock::closing::aggregate 完成".to_string(),
            );
        }
        Err(why) => {
            tracing::debug!(
                "Failed to event::taiwan_stock::closing::aggregate because {:?}",
                why
            );
        }
    }

    tracing::debug!("結束 event::taiwan_stock::closing::aggregate");
    tokio::time::sleep(Duration::from_secs(1)).await;
}

fn count(date: NaiveDate, market_id: i32, traded: i64) -> MarketTradedCount {
    MarketTradedCount {
        date,
        market_id,
        traded,
    }
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 3, d).unwrap()
}

/// 上櫃整天沒抓到（2026-03-17 只剩零量補值列）時判定不完整；上市正常不列入。
#[test]
fn shortfalls_flag_a_missing_market() {
    let counts = vec![
        count(day(13), 2, 1_010),
        count(day(13), 4, 880),
        count(day(16), 2, 1_005),
        count(day(16), 4, 875),
        count(day(17), 2, 1_002),
    ];
    assert_eq!(
        shortfalls(day(17), &counts),
        vec![Shortfall {
            market: "上櫃",
            traded: 0,
            baseline: 880,
        }]
    );
}

/// 冷門股沒成交造成的小幅波動不算不完整；低於近期最大值一半才算。
#[test]
fn shortfalls_tolerate_normal_variation() {
    let counts = vec![
        count(day(16), 2, 1_000),
        count(day(16), 4, 880),
        count(day(17), 2, 950),
        count(day(17), 4, 440),
    ];
    assert!(shortfalls(day(17), &counts).is_empty());

    let partial = vec![
        count(day(16), 4, 880),
        count(day(17), 2, 1_000),
        count(day(17), 4, 439),
    ];
    assert_eq!(shortfalls(day(17), &partial)[0].traded, 439);
}

/// 沒有近期資料可比較時，只要當天有成交就算完整；兩個市場都 0 檔則都列出。
#[test]
fn shortfalls_without_baseline() {
    assert!(shortfalls(day(17), &[count(day(17), 2, 10), count(day(17), 4, 5)]).is_empty());
    assert_eq!(shortfalls(day(17), &[]).len(), 2);
}

/// 臨時停市：上市、上櫃都 0 檔且沒有加權指數才成立；任一邊有成交或指數已寫入都不算。
#[test]
fn unscheduled_closure_requires_both_markets_and_index_missing() {
    let closed = shortfalls(day(17), &[count(day(16), 2, 1_000), count(day(16), 4, 880)]);
    assert!(is_unscheduled_closure(&closed, 0));
    assert!(!is_unscheduled_closure(&closed, 1));

    let listed_only = shortfalls(
        day(17),
        &[
            count(day(16), 2, 1_000),
            count(day(16), 4, 880),
            count(day(17), 4, 870),
        ],
    );
    assert_eq!(listed_only.len(), 1);
    assert!(!is_unscheduled_closure(&listed_only, 0));

    let partial = shortfalls(
        day(17),
        &[
            count(day(16), 2, 1_000),
            count(day(16), 4, 880),
            count(day(17), 2, 300),
            count(day(17), 4, 200),
        ],
    );
    assert_eq!(partial.len(), 2);
    assert!(!is_unscheduled_closure(&partial, 0));
    assert!(!is_unscheduled_closure(&[], 0));
}

/// 依序回傳筆數、記錄重算項目的假倉儲。
struct DerivedRepo {
    dates: Vec<NaiveDate>,
    counts: std::sync::Mutex<Vec<DerivedCounts>>,
    got_base: std::sync::Mutex<Option<NaiveDate>>,
}

impl DerivedRepo {
    fn new(dates: Vec<NaiveDate>, counts: Vec<DerivedCounts>) -> Self {
        Self {
            dates,
            counts: std::sync::Mutex::new(counts),
            got_base: std::sync::Mutex::new(None),
        }
    }
}

#[async_trait::async_trait]
impl ForwardReturnRepository for DerivedRepo {
    async fn fetch_trading_dates(&self, _until: NaiveDate, count: usize) -> Result<Vec<NaiveDate>> {
        Ok(self.dates.iter().copied().take(count).collect())
    }

    async fn upsert_forward_returns(
        &self,
        _base: NaiveDate,
        _end: NaiveDate,
        _horizon: usize,
    ) -> Result<u64> {
        Ok(0)
    }
}

#[async_trait::async_trait]
impl DerivedDataQuery for DerivedRepo {
    async fn count_derived(&self, _date: NaiveDate, base: NaiveDate) -> Result<DerivedCounts> {
        *self.got_base.lock().expect("測試鎖") = Some(base);
        Ok(self.counts.lock().expect("測試鎖").remove(0))
    }
}

/// 由新到舊的連續交易日，第一個是 `end`。
fn trading_days(end: NaiveDate, count: u64) -> Vec<NaiveDate> {
    (0..count).map(|i| end - chrono::Days::new(i)).collect()
}

const COMPLETE: DerivedCounts = DerivedCounts {
    signals: 1800,
    forward_returns: 1900,
    relative_strength: 1700,
};

/// 完整時不告警；非交易日不檢查；缺漏時只重算缺的項目，依重算後狀況決定標題。
#[tokio::test]
async fn check_derived_recomputes_missing_items() {
    let date = day(13);
    let recomputed = std::sync::Mutex::new(Vec::new());
    let recompute = |derived: Derived| {
        recomputed.lock().expect("測試鎖").push(derived);
        async move {
            match derived {
                Derived::Signals => Ok(()),
                Derived::ForwardReturns => Err(anyhow::anyhow!("連線中斷")),
                Derived::RelativeStrength => Ok(()),
            }
        }
    };

    let complete = DerivedRepo::new(trading_days(date, 6), vec![COMPLETE]);
    assert!(
        check_derived(&complete, date, recompute)
            .await
            .expect("檢查成功")
            .is_none()
    );
    assert_eq!(*complete.got_base.lock().expect("測試鎖"), Some(day(8)));

    let holiday = DerivedRepo::new(trading_days(day(12), 6), Vec::new());
    assert!(
        check_derived(&holiday, date, recompute)
            .await
            .expect("非交易日")
            .is_none()
    );

    let fixed = DerivedRepo::new(
        trading_days(date, 6),
        vec![
            DerivedCounts {
                signals: 0,
                ..COMPLETE
            },
            COMPLETE,
        ],
    );
    let (title, body) = check_derived(&fixed, date, recompute)
        .await
        .expect("檢查成功")
        .expect("有缺漏");
    assert_eq!(title, "收盤衍生資料已補算");
    assert!(
        body.contains(
            "檢查時缺：技術訊號
已重算"
        ),
        "{body}"
    );

    let still = DerivedRepo::new(
        trading_days(date, 6),
        vec![
            DerivedCounts::default(),
            DerivedCounts {
                forward_returns: 0,
                ..COMPLETE
            },
        ],
    );
    let (title, body) = check_derived(&still, date, recompute)
        .await
        .expect("檢查成功")
        .expect("有缺漏");
    assert_eq!(title, "收盤衍生資料仍缺漏");
    assert!(body.contains("重算失敗：未來報酬：連線中斷"), "{body}");
    assert_eq!(
        *recomputed.lock().expect("測試鎖"),
        [
            Derived::Signals,
            Derived::Signals,
            Derived::ForwardReturns,
            Derived::RelativeStrength
        ]
    );
}

/// 筆數為 0 的衍生資料才列為缺漏。
#[test]
fn missing_derived_lists_zero_counts() {
    assert!(missing_derived(COMPLETE).is_empty());
    assert_eq!(
        missing_derived(DerivedCounts::default()),
        [
            Derived::Signals,
            Derived::ForwardReturns,
            Derived::RelativeStrength
        ]
    );
    assert_eq!(
        missing_derived(DerivedCounts {
            forward_returns: 0,
            relative_strength: 0,
            ..COMPLETE
        }),
        [Derived::ForwardReturns, Derived::RelativeStrength]
    );
}

/// 衍生資料告警列出缺漏、重算結果與重算後狀況。
#[test]
fn derived_report_describes_before_rerun_and_after() {
    let both = [Derived::Signals, Derived::ForwardReturns];
    assert_eq!(
        derived_report(day(13), &both, &[], &[]),
        "2026-03-13 檢查時缺：技術訊號、未來報酬\n已重算\n重算後都已寫入"
    );
    let failed = derived_report(
        day(13),
        &both,
        &["技術訊號：連線中斷".to_owned()],
        &[Derived::Signals],
    );
    assert_eq!(
        failed,
        "2026-03-13 檢查時缺：技術訊號、未來報酬\n重算失敗：技術訊號：連線中斷\n重算後仍缺：技術訊號；請查看收盤匯總日誌，必要時手動回補"
    );
}

/// 告警內容依序說明檢查時的缺口、重跑結果與重跑後狀況。
#[test]
fn report_describes_before_rerun_and_after() {
    let missing = vec![Shortfall {
        market: "上櫃",
        traded: 0,
        baseline: 880,
    }];
    assert_eq!(
        report(day(17), &missing, None, &[]),
        "2026-03-17 檢查時：上櫃 0 檔有成交（近期 880 檔）\n已重跑收盤匯總\n重跑後兩個市場都已完整"
    );
    let text = report(day(17), &missing, Some("上櫃收盤資料 0 筆"), &missing);
    assert!(
        text.contains("重跑收盤匯總失敗：上櫃收盤資料 0 筆"),
        "{text}"
    );
    assert!(
        text.contains("重跑後仍不完整：上櫃 0 檔有成交（近期 880 檔）"),
        "{text}"
    );
}
