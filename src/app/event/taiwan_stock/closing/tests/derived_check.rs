//! 收盤衍生資料檢查的測試：缺漏判斷、只重算缺的那一步與報告內容。

use super::*;

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
