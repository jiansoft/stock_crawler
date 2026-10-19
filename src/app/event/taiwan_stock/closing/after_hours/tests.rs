//! 盤後資料完整性檢查的單元測試。

use std::sync::Mutex;

use super::*;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("固定日期合法")
}

/// 一天的筆數：`(日, 指數, 籌碼, 外資持股)`。
fn counts(d: u32, index: i64, chip: i64, foreign_holding: i64) -> AfterHoursCounts {
    AfterHoursCounts {
        date: day(d),
        index,
        chip,
        foreign_holding,
    }
}

/// 10-13～10-19 的正常筆數（由新到舊，第一筆是當天）。
fn normal_week(today: AfterHoursCounts) -> Vec<AfterHoursCounts> {
    let mut days = vec![today];
    days.extend([16, 15, 14, 13].map(|d| counts(d, 1, 2320, 2251)));
    days
}

/// 當天為 0 或低於近期最大值一半才列為缺漏；剛好一半不算；沒有近期資料時只看是否為 0。
#[test]
fn missing_after_hours_compares_with_recent_days() {
    assert!(missing_after_hours(&LATE_EVENING, &normal_week(counts(19, 1, 2317, 2260))).is_empty());
    assert_eq!(
        missing_after_hours(&LATE_EVENING, &normal_week(counts(19, 1, 0, 0))),
        [AfterHours::Chip, AfterHours::ForeignHolding]
    );
    assert_eq!(
        missing_after_hours(&LATE_EVENING, &normal_week(counts(19, 0, 1159, 1160))),
        [AfterHours::Index, AfterHours::Chip]
    );
    // 只檢查指定的項目。
    assert!(missing_after_hours(&WITH_CLOSING, &normal_week(counts(19, 1, 0, 0))).is_empty());
    assert_eq!(
        missing_after_hours(&LATE_EVENING, &[counts(19, 1, 10, 0)]),
        [AfterHours::ForeignHolding]
    );
    assert!(missing_after_hours(&LATE_EVENING, &[]).is_empty());
}

/// 依序回傳筆數的假倉儲。
struct AfterHoursRepo {
    dates: Vec<NaiveDate>,
    counts: Mutex<Vec<Vec<AfterHoursCounts>>>,
}

impl AfterHoursRepo {
    fn new(dates: Vec<NaiveDate>, counts: Vec<Vec<AfterHoursCounts>>) -> Self {
        Self {
            dates,
            counts: Mutex::new(counts),
        }
    }
}

#[async_trait::async_trait]
impl ForwardReturnRepository for AfterHoursRepo {
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
impl AfterHoursDataQuery for AfterHoursRepo {
    async fn count_after_hours(&self, dates: &[NaiveDate]) -> Result<Vec<AfterHoursCounts>> {
        assert_eq!(dates, self.dates.as_slice(), "以近期交易日查詢");
        Ok(self.counts.lock().expect("測試鎖").remove(0))
    }
}

/// 完整時不告警；當天沒有日線不檢查；缺漏時只重跑缺的項目，依重跑後狀況決定標題與內容。
#[tokio::test]
async fn check_after_hours_reruns_missing_items() {
    let date = day(19);
    let dates = vec![day(19), day(16), day(15), day(14), day(13)];
    let rerun = Mutex::new(Vec::new());
    let recompute = |item: AfterHours| {
        rerun.lock().expect("測試鎖").push(item);
        async move {
            match item {
                AfterHours::ForeignHolding => Err(anyhow::anyhow!("pool timed out")),
                _ => Ok(()),
            }
        }
    };

    let complete = AfterHoursRepo::new(dates.clone(), vec![normal_week(counts(19, 1, 2317, 2260))]);
    assert!(
        check_after_hours(&complete, date, &LATE_EVENING, recompute)
            .await
            .expect("檢查成功")
            .is_none()
    );

    // 收盤尚未寫入（最新交易日是 10-16）：不檢查，也不查筆數。
    let not_yet = AfterHoursRepo::new(dates[1..].to_vec(), Vec::new());
    assert!(
        check_after_hours(&not_yet, date, &LATE_EVENING, recompute)
            .await
            .expect("不檢查")
            .is_none()
    );

    let fixed = AfterHoursRepo::new(
        dates.clone(),
        vec![
            normal_week(counts(19, 0, 2317, 2260)),
            normal_week(counts(19, 1, 2317, 2260)),
        ],
    );
    let (title, body) = check_after_hours(&fixed, date, &LATE_EVENING, recompute)
        .await
        .expect("檢查成功")
        .expect("有缺漏");
    assert_eq!(title, "盤後資料已補齊");
    assert_eq!(
        body,
        "2026-10-19 檢查時缺：大盤指數\n已重跑\n重跑後都已寫入"
    );

    let still = AfterHoursRepo::new(
        dates,
        vec![
            normal_week(counts(19, 1, 0, 0)),
            normal_week(counts(19, 1, 2317, 0)),
        ],
    );
    let (title, body) = check_after_hours(&still, date, &LATE_EVENING, recompute)
        .await
        .expect("檢查成功")
        .expect("有缺漏");
    assert_eq!(title, "盤後資料仍缺漏");
    assert_eq!(
        body,
        "2026-10-19 檢查時缺：三大法人與融資融券、外資持股\n重跑失敗：外資持股：pool timed out\n\
         重跑後仍缺：外資持股；請確認來源是否已公布，必要時手動回補"
    );
    assert_eq!(
        *rerun.lock().expect("測試鎖"),
        [
            AfterHours::Index,
            AfterHours::Chip,
            AfterHours::ForeignHolding
        ]
    );
}

/// 官方本益比有補到日線才告警。
#[test]
fn official_ratio_report_only_when_rows_updated() {
    let mut summary = official_ratio::Summary {
        requests: 2,
        fetched_rows: 1973,
        ..Default::default()
    };
    assert!(official_ratio_report(day(19), &summary).is_none());
    summary.updated_rows = 28;
    summary.failed = 1;
    assert_eq!(
        official_ratio_report(day(19), &summary).as_deref(),
        Some(
            "2026-10-19 盤後檢查補上 28 筆日線的本益比／股價淨值比（官方 1973 筆、抓取失敗 1 個市場）"
        )
    );
}
