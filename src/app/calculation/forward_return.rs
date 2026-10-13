//! 收盤後回填未來報酬。
//!
//! 交易日 `end` 收盤時，對每個天期 N（[`FORWARD_HORIZONS`]）找出 `end` 往前第 N 個交易日，
//! 把它到 `end` 的還原報酬寫入 `daily_forward_return`。每天各天期只回填一個基準日，
//! 所以只要每個交易日都跑過一次，所有基準日的各天期都會補齊；重跑會覆寫同樣的值。

use anyhow::{Context, Result};
use chrono::NaiveDate;

use crate::domain::quote::forward_return::{self, FORWARD_HORIZONS, ForwardReturnRepository};
use crate::infra::database::repository::quote::PgQuoteRepository;

/// 回填 `end` 收盤時已滿各天期的基準日，回傳寫入的列數；`end` 不是交易日時不寫入。
///
/// # Errors
///
/// 查詢交易日或寫入失敗時回傳錯誤。
pub async fn calculate_forward_returns(end: NaiveDate) -> Result<u64> {
    run(&PgQuoteRepository::new(), end).await
}

/// [`calculate_forward_returns`] 的本體，倉儲可替換以便測試。
pub(crate) async fn run(repo: &impl ForwardReturnRepository, end: NaiveDate) -> Result<u64> {
    let longest = FORWARD_HORIZONS.iter().copied().max().unwrap_or_default();
    let dates = repo
        .fetch_trading_dates(end, longest + 1)
        .await
        .context("查詢交易日失敗")?;
    let mut written = 0;
    for horizon in FORWARD_HORIZONS {
        let Some(base) = forward_return::base_date(end, horizon, &dates) else {
            continue;
        };
        written += repo
            .upsert_forward_returns(base, end, horizon)
            .await
            .with_context(|| format!("寫入 {base} 的 {horizon} 日報酬失敗"))?;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
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
        async fn fetch_trading_dates(
            &self,
            _until: NaiveDate,
            count: usize,
        ) -> Result<Vec<NaiveDate>> {
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

    /// 交易日足夠時三個天期都回填，基準日是往前第 N 個交易日。
    #[tokio::test]
    async fn run_fills_every_horizon() {
        let end = day(13);
        let repo = FakeRepo {
            dates: calendar(end, 61),
            ..Default::default()
        };
        assert_eq!(run(&repo, end).await.expect("回填成功"), 30);
        let writes = repo.writes.lock().expect("測試鎖").clone();
        assert_eq!(
            writes,
            [
                (end - chrono::Days::new(5), end, 5),
                (end - chrono::Days::new(20), end, 20),
                (end - chrono::Days::new(60), end, 60),
            ]
        );
    }

    /// 交易日不足 60 日只回填較短天期；結束日不是交易日時都不寫。
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
}
