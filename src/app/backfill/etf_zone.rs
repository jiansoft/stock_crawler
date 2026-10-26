//! # ETF 專區資料
//!
//! stock_go `/stock/etfs` 讀的資料由這裡寫入：
//!
//! | 函式 | 內容 | 排程（台北） |
//! |------|------|--------------|
//! | [`execute_daily`] | 證交所 MIS 預估淨值 → 補推斷分類 → 含息績效 → 重建快照 | 週一～五 17:30 |
//! | [`execute_weekly_profiles`] | Yahoo ETF 頁的基本資料、分類與持股 → 重建快照 | 週六 12:10 |
//! | [`refresh_snapshot`] | 只重建快照（21:50 籌碼入庫後帶進外資買賣超） | 週一～五 22:05 |
//! | [`fill_recent_nav_gaps`] | MoneyDJ 近 30 天淨值 → 補缺日與空欄 → 重建快照 | 週六 13:05 |
//! | [`backfill_nav_history`] | MoneyDJ 歷史淨值與市價 → 重建快照（補 MIS 上線前的日子） | 手動 |
//! | [`backfill_nav_history_since`] | 同上，起日改為指定日期（例如 2019-01-01） | 手動 |

use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{Local, Months, NaiveDate};
use rand::RngExt;

use crate::{
    app::calculation,
    core::util::datetime::Weekend,
    domain::etf::{ClassifyHint, EtfNavHistoryPoint, EtfProfileRecord, EtfRepository, classify},
    infra::{
        crawler::{moneydj, twse, yahoo},
        database::repository::etf::PgEtfRepository,
    },
};

/// 每累積這麼多檔 Yahoo 基本資料就寫入一次，任務中途中止也不會全部白抓。
const PROFILE_BATCH: usize = 50;

/// 每週補缺日往回看的天數（日曆天）。
const GAP_FILL_DAYS: i64 = 30;

/// 每個交易日：淨值、推斷分類、績效與快照。淨值抓取失敗時其餘步驟照做，最後回傳錯誤以觸發告警。
///
/// # Errors
///
/// 淨值抓取或任一步驟的資料庫操作失敗時回傳錯誤。
pub async fn execute_daily() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }
    let repo = PgEtfRepository::new();
    let nav_result = match twse::etf_nav::visit().await {
        Ok(records) => {
            let written = repo.save_navs(&records).await?;
            tracing::info!(
                "ETF 預估淨值入庫: records={}, written={written}",
                records.len()
            );
            Ok(())
        }
        Err(why) => Err(why),
    };

    let listed = repo.listed_etfs().await?;
    let heuristics: Vec<EtfProfileRecord> = listed
        .iter()
        .map(|(symbol, name)| heuristic_profile(symbol, name))
        .collect();
    let profiles = repo.insert_missing_profiles(&heuristics).await?;
    calculation::etf_performance::execute().await?;
    let rows = repo.refresh_snapshot().await?;
    tracing::info!(
        "ETF 專區每日更新: etfs={}, heuristic_profiles={profiles}, snapshot_rows={rows}",
        listed.len()
    );
    nav_result.context("抓取 ETF 預估淨值失敗")
}

/// 每週：逐檔抓 Yahoo ETF 頁，寫入基本資料、分類與持股，最後重建快照。
///
/// # Errors
///
/// 資料庫操作失敗，或超過一半的 ETF 抓取失敗（多半是 Yahoo 改版或封鎖）時回傳錯誤。
pub async fn execute_weekly_profiles() -> Result<()> {
    let repo = PgEtfRepository::new();
    let listed = repo.listed_etfs().await?;
    let (mut saved, mut not_found, mut failed) = (0usize, 0usize, 0usize);
    let mut batch: Vec<EtfProfileRecord> = Vec::with_capacity(PROFILE_BATCH);

    for (index, (symbol, name)) in listed.iter().enumerate() {
        if index > 0 {
            request_interval().await;
        }
        match yahoo::etf::visit(symbol).await {
            Ok(Some(etf)) => {
                if let Err(why) = repo.replace_portfolio(&etf.portfolio).await {
                    tracing::warn!("ETF 持股寫入失敗: stock_symbol={symbol}, error={why:#}");
                }
                batch.push(classified(etf.profile, name));
            }
            Ok(None) => not_found += 1,
            Err(why) => {
                failed += 1;
                tracing::warn!("Yahoo ETF 頁抓取失敗: stock_symbol={symbol}, error={why:#}");
            }
        }
        if batch.len() >= PROFILE_BATCH {
            saved += save_batch(&repo, &mut batch).await?;
        }
    }
    saved += save_batch(&repo, &mut batch).await?;
    let rows = repo.refresh_snapshot().await?;
    tracing::info!(
        "ETF 基本資料更新: etfs={}, saved={saved}, not_found={not_found}, failed={failed}, snapshot_rows={rows}",
        listed.len()
    );
    if too_many_failures(failed, listed.len()) {
        bail!("Yahoo ETF 頁抓取失敗 {failed}／{} 檔", listed.len());
    }
    Ok(())
}

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
    let (from, to) = nav_history_since_range(Local::now().date_naive(), from)?;
    backfill_nav_range(from, to).await
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

/// 只重建快照。
///
/// # Errors
///
/// 資料庫操作失敗時回傳錯誤。
pub async fn refresh_snapshot() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }
    let rows = PgEtfRepository::new().refresh_snapshot().await?;
    tracing::info!("ETF 快照重建: rows={rows}");
    Ok(())
}

async fn save_batch(repo: &PgEtfRepository, batch: &mut Vec<EtfProfileRecord>) -> Result<usize> {
    if batch.is_empty() {
        return Ok(0);
    }
    repo.save_profiles(batch).await?;
    let count = batch.len();
    batch.clear();
    Ok(count)
}

/// 失敗超過一半視為來源整體出問題（改版或封鎖），需要告警。
fn too_many_failures(failed: usize, total: usize) -> bool {
    total > 0 && failed * 2 > total
}

/// 沒有來源資料時，只靠代號與名稱推得的基本資料。
pub(crate) fn heuristic_profile(symbol: &str, name: &str) -> EtfProfileRecord {
    let classification = classify(symbol, name, &ClassifyHint::default());
    EtfProfileRecord {
        stock_symbol: symbol.to_string(),
        strategy: classification.strategy.code().to_string(),
        region: classification.region,
        themes: classification.themes,
        source: "heuristic".to_string(),
        ..Default::default()
    }
}

/// 以來源提供的類型、主動式標記、地區與追蹤指數輔助推斷，填上分類欄位。
pub(crate) fn classified(profile: EtfProfileRecord, name: &str) -> EtfProfileRecord {
    let classification = classify(
        &profile.stock_symbol,
        name,
        &ClassifyHint {
            category: &profile.category,
            is_active: profile.is_active,
            region: &profile.region,
            benchmark: &profile.benchmark,
        },
    );
    EtfProfileRecord {
        strategy: classification.strategy.code().to_string(),
        region: classification.region,
        themes: classification.themes,
        ..profile
    }
}

/// ETF 之間隨機停 1.5～3 秒，與 Yahoo 財報採集相同的節奏。
async fn request_interval() {
    let ms = rand::rng().random_range(1500..=3000);
    tokio::time::sleep(Duration::from_millis(ms)).await;
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

    #[test]
    fn heuristic_profile_uses_symbol_and_name_only() {
        let profile = heuristic_profile("00679B", "元大美債20年");
        assert_eq!(profile.strategy, "bond");
        assert_eq!(profile.region, "美國");
        assert_eq!(profile.source, "heuristic");
        assert!(profile.full_name.is_empty());
    }

    #[test]
    fn classified_profile_takes_source_hints() {
        let source = EtfProfileRecord {
            stock_symbol: "00893".to_string(),
            region: "全球".to_string(),
            category: "指數型".to_string(),
            full_name: "國泰全球智能電動車ETF".to_string(),
            source: "yahoo".to_string(),
            ..Default::default()
        };
        let profile = classified(source, "國泰智能電動車");
        assert_eq!(profile.strategy, "overseas");
        assert_eq!(profile.region, "全球");
        assert_eq!(
            profile.themes,
            vec!["人工智慧".to_string(), "電動車".to_string()]
        );
        assert_eq!(profile.full_name, "國泰全球智能電動車ETF");
        assert_eq!(profile.source, "yahoo");

        let active = EtfProfileRecord {
            stock_symbol: "00981A".to_string(),
            is_active: true,
            ..Default::default()
        };
        assert_eq!(classified(active, "主動統一台股增長").strategy, "active");
    }

    #[test]
    fn too_many_failures_means_more_than_half() {
        assert!(!too_many_failures(0, 0));
        assert!(!too_many_failures(180, 360));
        assert!(too_many_failures(181, 360));
    }
}
