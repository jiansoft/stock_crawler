//! # 除權息日漲跌修正
//!
//! 每日收盤行情在除權息當天的漲跌是 0（交易所標「X」），這裡以官方除權除息計算結果表的
//! 開盤競價基準重算（原因見 [`crate::domain::quote::ex_right`]）。
//!
//! | 函式 | 用途 |
//! |------|------|
//! | [`execute`] | 收盤流程：報價寫入後、均線與統計表之前，修正當天的除權息股 |
//! | [`backfill_range`] | 手動回補：按年抓取一段期間，修正日線並同步調整市場廣度統計表 |
//!
//! 兩個市場分別抓取；單一市場失敗只記錄、不影響另一個市場與收盤流程的其他步驟。

use std::{future::Future, time::Duration};

use anyhow::{Result, bail};
use chrono::{Datelike, NaiveDate};

use crate::{
    domain::quote::ex_right::{ExRightChangeRepository, ExRightReference},
    infra::{
        crawler::{tpex, twse},
        database::repository::quote::PgQuoteRepository,
    },
};

/// 歷史回補兩次請求之間的停頓，避免被交易所擋。
const REQUEST_PAUSE: Duration = Duration::from_secs(3);

/// 資料來源市場。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Market {
    /// 上市，來源 TWSE TWT49U。
    Listed,
    /// 上櫃，來源 TPEx exDailyQ。
    Otc,
}

impl Market {
    fn label(self) -> &'static str {
        match self {
            Self::Listed => "上市",
            Self::Otc => "上櫃",
        }
    }
}

/// 一次修正的結果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// 抓取的（市場, 期間）組數。
    pub requests: usize,
    /// 抓取失敗的組數。
    pub failed: usize,
    /// 官方回傳的基準價筆數。
    pub fetched: usize,
    /// 實際更新的日線列數。
    pub updated: u64,
}

/// 收盤流程入口：修正 `date` 當天除權息股的漲跌，不調整統計表（統計表在之後才寫入）。
///
/// # Errors
///
/// 寫入失敗，或兩個市場都抓取失敗時回傳錯誤。
pub async fn execute(date: NaiveDate) -> Result<Summary> {
    run(
        &PgQuoteRepository::new(),
        &[(date, date)],
        false,
        fetch,
        || async {},
    )
    .await
}

/// 手動回補：逐年修正 `from`～`to` 的除權息日漲跌，並同步調整市場廣度統計表的漲跌家數。
/// 重複執行安全（已修正的列漲跌不為 0，不會再動）。
///
/// # Errors
///
/// 寫入失敗，或有發請求卻全部失敗時回傳錯誤。
pub async fn backfill_range(from: NaiveDate, to: NaiveDate) -> Result<Summary> {
    run(
        &PgQuoteRepository::new(),
        &yearly_windows(from, to),
        true,
        fetch,
        || tokio::time::sleep(REQUEST_PAUSE),
    )
    .await
}

/// 把 `from`～`to` 切成不跨年的區間（交易所一次查詢以一年為宜）。
fn yearly_windows(from: NaiveDate, to: NaiveDate) -> Vec<(NaiveDate, NaiveDate)> {
    let mut windows = Vec::new();
    let mut start = from;
    while start <= to {
        let year_end = NaiveDate::from_ymd_opt(start.year(), 12, 31).unwrap_or(to);
        let end = year_end.min(to);
        windows.push((start, end));
        let Some(next) = end.succ_opt() else {
            break;
        };
        start = next;
    }
    windows
}

/// 依市場呼叫對應的爬蟲。
async fn fetch(market: Market, from: NaiveDate, to: NaiveDate) -> Result<Vec<ExRightReference>> {
    match market {
        Market::Listed => twse::ex_right_result::visit_references(from, to).await,
        Market::Otc => tpex::ex_right_result::visit_references(from, to).await,
    }
}

/// 修正流程本體；`fetch` 與 `pause` 可注入，測試時換成假資料與不等待。
async fn run<F, FF, P, PF>(
    repo: &impl ExRightChangeRepository,
    windows: &[(NaiveDate, NaiveDate)],
    with_stats: bool,
    fetch: F,
    pause: P,
) -> Result<Summary>
where
    F: Fn(Market, NaiveDate, NaiveDate) -> FF,
    FF: Future<Output = Result<Vec<ExRightReference>>>,
    P: Fn() -> PF,
    PF: Future<Output = ()>,
{
    let mut summary = Summary::default();
    for &(from, to) in windows {
        for market in [Market::Listed, Market::Otc] {
            if summary.requests > 0 {
                pause().await;
            }
            summary.requests += 1;
            let references = match fetch(market, from, to).await {
                Ok(references) => references,
                Err(why) => {
                    summary.failed += 1;
                    tracing::warn!(
                        "抓取{}除權息基準價失敗 {from}~{to}: {why:#}",
                        market.label()
                    );
                    continue;
                }
            };
            summary.fetched += references.len();
            let updated = repo
                .apply_ex_right_references(&references, with_stats)
                .await?;
            summary.updated += updated;
            tracing::info!(
                "{}除權息日漲跌修正 {from}~{to}: 基準價 {} 筆、更新日線 {updated} 列",
                market.label(),
                references.len()
            );
        }
    }
    if summary.requests > 0 && summary.failed == summary.requests {
        bail!("除權息基準價全部抓取失敗（{} 次）", summary.requests);
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use anyhow::anyhow;
    use async_trait::async_trait;
    use rust_decimal_macros::dec;

    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("有效日期")
    }

    /// 記錄寫入參數的假倉儲；每筆基準價算更新一列。
    #[derive(Default)]
    struct FakeRepo {
        calls: Mutex<Vec<(Vec<String>, bool)>>,
        fail: bool,
    }

    #[async_trait]
    impl ExRightChangeRepository for FakeRepo {
        async fn apply_ex_right_references(
            &self,
            references: &[ExRightReference],
            with_stats: bool,
        ) -> Result<u64> {
            if self.fail {
                return Err(anyhow!("寫入失敗"));
            }
            self.calls.lock().expect("測試鎖").push((
                references.iter().map(|r| r.stock_symbol.clone()).collect(),
                with_stats,
            ));
            Ok(references.len() as u64)
        }
    }

    fn reference(symbol: &str, date: NaiveDate) -> ExRightReference {
        ExRightReference {
            stock_symbol: symbol.to_owned(),
            date,
            reference_price: dec!(10),
        }
    }

    /// 跨年的期間切成每年一段，頭尾依實際起訖日。
    #[test]
    fn yearly_windows_split_by_calendar_year() {
        assert_eq!(
            yearly_windows(day(2023, 6, 1), day(2025, 2, 3)),
            [
                (day(2023, 6, 1), day(2023, 12, 31)),
                (day(2024, 1, 1), day(2024, 12, 31)),
                (day(2025, 1, 1), day(2025, 2, 3)),
            ]
        );
        assert_eq!(
            yearly_windows(day(2026, 10, 13), day(2026, 10, 13)),
            [(day(2026, 10, 13), day(2026, 10, 13))]
        );
        assert!(yearly_windows(day(2026, 10, 14), day(2026, 10, 13)).is_empty());
    }

    /// 每段期間依序抓上市、上櫃並寫入；第一次之後才停頓；一個市場失敗只記錄。
    #[tokio::test]
    async fn run_fetches_both_markets_and_skips_failures() {
        let repo = FakeRepo::default();
        let pauses = Mutex::new(0);
        let windows = [
            (day(2024, 1, 1), day(2024, 12, 31)),
            (day(2025, 1, 1), day(2025, 3, 1)),
        ];
        let summary = run(
            &repo,
            &windows,
            true,
            |market, from, _to| async move {
                match (market, from.year()) {
                    (Market::Otc, 2024) => Err(anyhow!("逾時")),
                    (Market::Listed, year) => Ok(vec![reference(&format!("L{year}"), from)]),
                    (Market::Otc, year) => Ok(vec![
                        reference(&format!("O{year}"), from),
                        reference(&format!("P{year}"), from),
                    ]),
                }
            },
            || {
                *pauses.lock().expect("測試鎖") += 1;
                async {}
            },
        )
        .await
        .expect("部分失敗仍成功");

        assert_eq!(
            summary,
            Summary {
                requests: 4,
                failed: 1,
                fetched: 4,
                updated: 4,
            }
        );
        assert_eq!(*pauses.lock().expect("測試鎖"), 3);
        let calls = repo.calls.lock().expect("測試鎖");
        let symbols: Vec<Vec<String>> = calls.iter().map(|(s, _)| s.clone()).collect();
        assert_eq!(
            symbols,
            [
                vec!["L2024".to_owned()],
                vec!["L2025".to_owned()],
                vec!["O2025".to_owned(), "P2025".to_owned()],
            ]
        );
        assert!(calls.iter().all(|(_, with_stats)| *with_stats));
    }

    /// 全部抓取失敗回錯誤；寫入失敗直接回錯誤。
    #[tokio::test]
    async fn run_fails_when_all_fetches_fail_or_write_fails() {
        let windows = [(day(2026, 10, 13), day(2026, 10, 13))];
        let error = run(
            &FakeRepo::default(),
            &windows,
            false,
            |_, _, _| async { Err(anyhow!("逾時")) },
            || async {},
        )
        .await
        .expect_err("全部失敗");
        assert!(
            error.to_string().contains("全部抓取失敗（2 次）"),
            "{error}"
        );

        let failing = FakeRepo {
            fail: true,
            ..Default::default()
        };
        let error = run(
            &failing,
            &windows,
            false,
            |_, from, _| async move { Ok(vec![reference("2330", from)]) },
            || async {},
        )
        .await
        .expect_err("寫入失敗");
        assert!(error.to_string().contains("寫入失敗"), "{error}");
    }
}
