//! 收盤後回填未來報酬的測試。

use std::sync::Mutex;

use anyhow::anyhow;
use async_trait::async_trait;

use super::*;

/// 交易日固定、記錄寫入參數的假倉儲；`fail_horizon` 的天期寫入時回錯誤。
#[derive(Default)]
struct FakeRepo {
    dates: Vec<NaiveDate>,
    writes: Mutex<Vec<(NaiveDate, NaiveDate, usize)>>,
    fail_horizon: Option<usize>,
}

#[async_trait]
impl ForwardReturnRepository for FakeRepo {
    async fn fetch_trading_dates(&self, _until: NaiveDate, count: usize) -> Result<Vec<NaiveDate>> {
        Ok(self.dates.iter().copied().take(count).collect())
    }

    async fn upsert_forward_returns(
        &self,
        base: NaiveDate,
        end: NaiveDate,
        horizon: usize,
    ) -> Result<u64> {
        if self.fail_horizon == Some(horizon) {
            return Err(anyhow!("連線中斷"));
        }
        self.writes
            .lock()
            .expect("測試鎖")
            .push((base, end, horizon));
        Ok(10)
    }
}

/// 由新到舊連續 `count` 個交易日（每天一個），第一個是 `end`。
fn calendar(end: NaiveDate, count: u64) -> Vec<NaiveDate> {
    (0..count).map(|i| end - chrono::Days::new(i)).collect()
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("有效日期")
}

/// 交易日足夠時每個天期都回填，基準日是往前第 N 個交易日。
#[tokio::test]
async fn run_fills_every_horizon() {
    let end = day(13);
    let repo = FakeRepo {
        dates: calendar(end, 241),
        ..Default::default()
    };
    assert_eq!(run(&repo, end).await.expect("回填成功"), 50);
    let writes = repo.writes.lock().expect("測試鎖").clone();
    assert_eq!(
        writes,
        [
            (end - chrono::Days::new(5), end, 5),
            (end - chrono::Days::new(20), end, 20),
            (end - chrono::Days::new(60), end, 60),
            (end - chrono::Days::new(120), end, 120),
            (end - chrono::Days::new(240), end, 240),
        ]
    );
}

/// 交易日不足的天期不回填；結束日不是交易日時都不寫。
#[tokio::test]
async fn run_skips_horizons_without_enough_history() {
    let end = day(13);
    let short = FakeRepo {
        dates: calendar(end, 30),
        ..Default::default()
    };
    assert_eq!(run(&short, end).await.expect("回填成功"), 20);

    let holiday = FakeRepo {
        dates: calendar(day(12), 61),
        ..Default::default()
    };
    assert_eq!(run(&holiday, end).await.expect("非交易日"), 0);
    assert!(holiday.writes.lock().expect("測試鎖").is_empty());
}

/// 寫入失敗回錯誤並指出基準日與天期。
#[tokio::test]
async fn run_reports_failed_horizon() {
    let end = day(13);
    let repo = FakeRepo {
        dates: calendar(end, 61),
        fail_horizon: Some(20),
        ..Default::default()
    };
    let error = run(&repo, end).await.expect_err("寫入失敗");
    assert!(
        format!("{error:#}").contains("寫入 2026-09-23 的 20 日報酬失敗"),
        "{error:#}"
    );
}
