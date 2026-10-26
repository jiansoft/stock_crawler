//! ETF 淨值歷史回補：MoneyDJ 淨值與市價 → `etf_daily_nav` 缺日 → 重建快照。
//!
//! 排程與手動入口見上層 [`super`] 的表格。

use anyhow::{Context, Result, bail};
use chrono::{Local, Months, NaiveDate};

use super::{request_interval, too_many_failures};
use crate::{
    domain::etf::{EtfNavHistoryPoint, EtfRepository},
    infra::{crawler::moneydj, database::repository::etf::PgEtfRepository},
};

/// 每週補缺日往回看的天數（日曆天）。
const GAP_FILL_DAYS: i64 = 30;

/// 逐檔抓 MoneyDJ 最近 `years` 年到昨天的淨值與市價，補進 `etf_daily_nav` 還沒有的日期，最後重建快照
/// （近 20 筆折溢價平均跟著有歷史可算）。今天交給 MIS：MoneyDJ 當天的淨值可能還是前一天的數字。
///
/// # Errors
///
/// 資料庫操作失敗，或超過一半的 ETF 抓取失敗時回傳錯誤。
pub async fn backfill_nav_history(years: u32) -> Result<()> {
    let (from, to) =
        nav_history_range(Local::now().date_naive(), years).context("回補起日超出範圍")?;
    backfill_nav_range(from, to).await
}

/// 同 [`backfill_nav_history`]，起日改為指定日期（含）到昨天；起日晚於昨天時回傳錯誤。
///
/// # Errors
///
/// 起日不早於今天、資料庫操作失敗，或超過一半的 ETF 抓取失敗時回傳錯誤。
pub async fn backfill_nav_history_since(from: NaiveDate) -> Result<()> {
    backfill_nav_since_with(
        &PgEtfRepository::new(),
        Local::now().date_naive(),
        from,
        |from, to| move |symbol| async move { moneydj::etf_nav::visit(&symbol, from, to).await },
        request_interval,
    )
    .await
}

/// [`backfill_nav_history_since`] 的可注入版本：先算 `from` 到昨天的區間，再交給 `make_fetch` 產生逐檔抓取。
async fn backfill_nav_since_with<R, Make, Fetch, FetchFut, Pause, PauseFut>(
    repo: &R,
    today: NaiveDate,
    from: NaiveDate,
    make_fetch: Make,
    pause: Pause,
) -> Result<()>
where
    R: EtfRepository,
    Make: FnOnce(NaiveDate, NaiveDate) -> Fetch,
    Fetch: FnMut(String) -> FetchFut,
    FetchFut: Future<Output = Result<Vec<EtfNavHistoryPoint>>>,
    Pause: FnMut() -> PauseFut,
    PauseFut: Future<Output = ()>,
{
    let (from, to) = nav_history_since_range(today, from)?;
    backfill_nav_range_with(repo, from, to, make_fetch(from, to), pause).await
}

/// 每週：以 MoneyDJ 近 30 天的淨值補上缺日（MIS 排程失敗的日子）與空欄（正式淨值還沒回填的列）。
///
/// # Errors
///
/// 資料庫操作失敗，或超過一半的 ETF 抓取失敗時回傳錯誤。
pub async fn fill_recent_nav_gaps() -> Result<()> {
    let (from, to) = recent_range(Local::now().date_naive()).context("補缺日區間超出範圍")?;
    backfill_nav_range(from, to).await
}

async fn backfill_nav_range(from: NaiveDate, to: NaiveDate) -> Result<()> {
    backfill_nav_range_with(
        &PgEtfRepository::new(),
        from,
        to,
        |symbol| async move { moneydj::etf_nav::visit(&symbol, from, to).await },
        request_interval,
    )
    .await
}

/// 逐檔回補上市 ETF 的淨值，最後重建快照；倉儲、抓取與停頓由呼叫端注入。
async fn backfill_nav_range_with<R, Fetch, FetchFut, Pause, PauseFut>(
    repo: &R,
    from: NaiveDate,
    to: NaiveDate,
    fetch: Fetch,
    pause: Pause,
) -> Result<()>
where
    R: EtfRepository,
    Fetch: FnMut(String) -> FetchFut,
    FetchFut: Future<Output = Result<Vec<EtfNavHistoryPoint>>>,
    Pause: FnMut() -> PauseFut,
    PauseFut: Future<Output = ()>,
{
    let listed = repo.listed_etfs().await?;
    let symbols: Vec<&str> = listed.iter().map(|(symbol, _)| symbol.as_str()).collect();
    let tally = backfill_symbols(
        &symbols,
        fetch,
        |symbol, points| async move { repo.insert_nav_history(&symbol, &points).await },
        pause,
    )
    .await?;
    let rows = repo.refresh_snapshot().await?;
    tracing::info!(
        "ETF 歷史淨值回補: etfs={}, from={from}, to={to}, inserted={}, empty={}, failed={}, snapshot_rows={rows}",
        symbols.len(),
        tally.inserted,
        tally.empty,
        tally.failed
    );
    if too_many_failures(tally.failed, symbols.len()) {
        bail!(
            "MoneyDJ ETF 淨值抓取失敗 {}／{} 檔",
            tally.failed,
            symbols.len()
        );
    }
    Ok(())
}

/// 逐檔回補的統計。
#[derive(Debug, Default, PartialEq, Eq)]
struct NavHistoryTally {
    /// 新增的列數。
    inserted: u64,
    /// 來源查無資料的檔數。
    empty: usize,
    /// 抓取失敗的檔數。
    failed: usize,
}

/// 逐檔抓取並寫入；檔與檔之間呼叫 `pause`。抓取失敗只記錄並略過，寫入失敗則中止。
///
/// 參數都傳擁有權而不是借用：借用參數的 async closure 產生的 future 無法證明是 `Send`，
/// 排程任務（`create_job`）要求 `Send`。
async fn backfill_symbols<Fetch, FetchFut, Save, SaveFut, Pause, PauseFut>(
    symbols: &[&str],
    mut fetch: Fetch,
    mut save: Save,
    mut pause: Pause,
) -> Result<NavHistoryTally>
where
    Fetch: FnMut(String) -> FetchFut,
    FetchFut: Future<Output = Result<Vec<EtfNavHistoryPoint>>>,
    Save: FnMut(String, Vec<EtfNavHistoryPoint>) -> SaveFut,
    SaveFut: Future<Output = Result<u64>>,
    Pause: FnMut() -> PauseFut,
    PauseFut: Future<Output = ()>,
{
    let mut tally = NavHistoryTally::default();
    for (index, symbol) in symbols.iter().enumerate() {
        if index > 0 {
            pause().await;
        }
        match fetch(symbol.to_string()).await {
            Ok(points) if points.is_empty() => tally.empty += 1,
            Ok(points) => tally.inserted += save(symbol.to_string(), points).await?,
            Err(why) => {
                tally.failed += 1;
                tracing::warn!("MoneyDJ ETF 淨值抓取失敗: stock_symbol={symbol}, error={why:#}");
            }
        }
    }
    Ok(tally)
}

/// 補缺日區間：[`GAP_FILL_DAYS`] 天前到昨天。
fn recent_range(today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    Some((
        today.checked_sub_signed(chrono::Duration::days(GAP_FILL_DAYS))?,
        today.pred_opt()?,
    ))
}

/// 回補區間：`years` 年前的今天到昨天。
fn nav_history_range(today: NaiveDate, years: u32) -> Option<(NaiveDate, NaiveDate)> {
    let from = today.checked_sub_months(Months::new(years.checked_mul(12)?))?;
    Some((from, today.pred_opt()?))
}

/// 回補區間：`from` 到昨天；`from` 不早於今天時沒有區間，回傳錯誤。
fn nav_history_since_range(today: NaiveDate, from: NaiveDate) -> Result<(NaiveDate, NaiveDate)> {
    let to = today.pred_opt().context("今天沒有前一天")?;
    if from > to {
        bail!("回補起日 {from} 必須早於今天 {today}");
    }
    Ok((from, to))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn backfill_symbols_counts_and_skips_failures() {
        let point = EtfNavHistoryPoint {
            date: NaiveDate::from_ymd_opt(2026, 10, 21).expect("日期應合法"),
            nav: rust_decimal::Decimal::ONE,
            market_price: None,
        };
        let mut saved: Vec<String> = Vec::new();
        let mut pauses = 0;
        let tally = backfill_symbols(
            &["0050", "9999", "00679B"],
            |symbol| {
                let result = match symbol.as_str() {
                    "0050" => Ok(vec![point.clone(), point.clone()]),
                    "9999" => Ok(Vec::new()),
                    _ => Err(anyhow::anyhow!("逾時")),
                };
                async move { result }
            },
            |symbol, points| {
                saved.push(symbol);
                async move { Ok(points.len() as u64) }
            },
            || {
                pauses += 1;
                async {}
            },
        )
        .await
        .expect("抓取失敗不應中止");
        assert_eq!(
            tally,
            NavHistoryTally {
                inserted: 2,
                empty: 1,
                failed: 1
            }
        );
        assert_eq!(saved, vec!["0050"]);
        assert_eq!(pauses, 2);

        let result = backfill_symbols(
            &["0050"],
            |_| {
                let points = vec![point.clone()];
                async move { Ok(points) }
            },
            |_, _| async { Err(anyhow::anyhow!("連線中斷")) },
            || async {},
        )
        .await;
        assert!(result.is_err(), "寫入失敗要中止");
    }

    #[tokio::test]
    async fn backfill_range_writes_rebuilds_snapshot_and_alerts_on_mass_failure() {
        use crate::domain::etf::test_double::RecordingEtfRepository;

        let day = NaiveDate::from_ymd_opt(2026, 10, 21).expect("日期應合法");
        let point = EtfNavHistoryPoint {
            date: day,
            nav: rust_decimal::Decimal::ONE,
            market_price: None,
        };
        let repo = RecordingEtfRepository::with_listed(&["0050", "0056", "00679B"]);
        backfill_nav_range_with(
            &repo,
            day,
            day,
            |symbol| {
                let result = if symbol == "00679B" {
                    Err(anyhow::anyhow!("逾時"))
                } else {
                    Ok(vec![point.clone()])
                };
                async move { result }
            },
            || async {},
        )
        .await
        .expect("一檔失敗不算整體失敗");
        assert_eq!(
            repo.inserted(),
            vec![("0050".to_string(), 1), ("0056".to_string(), 1)]
        );
        assert_eq!(repo.snapshots(), 1);

        let failing = RecordingEtfRepository::with_listed(&["0050", "0056"]);
        let result = backfill_nav_range_with(
            &failing,
            day,
            day,
            |_| async { Err(anyhow::anyhow!("封鎖")) },
            || async {},
        )
        .await;
        assert!(result.is_err(), "超過一半失敗要告警");
        assert_eq!(failing.snapshots(), 1, "失敗也照樣重建快照");
    }

    #[tokio::test]
    async fn backfill_since_passes_range_to_fetch_and_rejects_future_start() {
        use crate::domain::etf::test_double::RecordingEtfRepository;
        use std::sync::{Arc, Mutex};

        let today = NaiveDate::from_ymd_opt(2026, 10, 26).expect("日期應合法");
        let from = NaiveDate::from_ymd_opt(2019, 1, 1).expect("日期應合法");
        let point = EtfNavHistoryPoint {
            date: from,
            nav: rust_decimal::Decimal::ONE,
            market_price: None,
        };
        let seen = Arc::new(Mutex::new(None));
        let repo = RecordingEtfRepository::with_listed(&["0050"]);
        let recorder = Arc::clone(&seen);
        backfill_nav_since_with(
            &repo,
            today,
            from,
            move |from, to| {
                *recorder.lock().expect("鎖不應中毒") = Some((from, to));
                move |_| {
                    let points = vec![point.clone()];
                    async move { Ok(points) }
                }
            },
            || async {},
        )
        .await
        .expect("起日早於今天應成功");
        assert_eq!(
            *seen.lock().expect("鎖不應中毒"),
            Some((
                from,
                NaiveDate::from_ymd_opt(2026, 10, 25).expect("日期應合法")
            ))
        );
        assert_eq!(repo.inserted(), vec![("0050".to_string(), 1)]);

        let untouched = RecordingEtfRepository::with_listed(&["0050"]);
        let result = backfill_nav_since_with(
            &untouched,
            today,
            today,
            |_, _| |_| async { Ok(Vec::new()) },
            || async {},
        )
        .await;
        assert!(result.is_err(), "起日不早於今天要回傳錯誤");
        assert_eq!(untouched.snapshots(), 0, "起日不合法時不抓也不重建快照");
    }

    #[test]
    fn nav_history_since_range_runs_to_yesterday() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 26).expect("日期應合法");
        let from = NaiveDate::from_ymd_opt(2019, 1, 1).expect("日期應合法");
        let yesterday = NaiveDate::from_ymd_opt(2026, 10, 25).expect("日期應合法");
        assert_eq!(
            nav_history_since_range(today, from).expect("起日早於今天應有區間"),
            (from, yesterday)
        );
        assert_eq!(
            nav_history_since_range(today, yesterday).expect("起日等於昨天應有區間"),
            (yesterday, yesterday)
        );
        let err = nav_history_since_range(today, today).expect_err("起日不早於今天應回傳錯誤");
        assert!(err.to_string().contains("必須早於今天"), "{err}");
        assert!(nav_history_since_range(NaiveDate::MIN, from).is_err());
    }

    #[test]
    fn nav_history_range_ends_yesterday() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 22).expect("日期應合法");
        assert_eq!(
            nav_history_range(today, 3),
            Some((
                NaiveDate::from_ymd_opt(2023, 10, 22).expect("日期應合法"),
                NaiveDate::from_ymd_opt(2026, 10, 21).expect("日期應合法"),
            ))
        );
        assert_eq!(nav_history_range(today, u32::MAX), None);
        assert_eq!(
            recent_range(today),
            Some((
                NaiveDate::from_ymd_opt(2026, 9, 22).expect("日期應合法"),
                NaiveDate::from_ymd_opt(2026, 10, 21).expect("日期應合法"),
            ))
        );
    }
}
