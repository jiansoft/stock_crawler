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
use chrono::Local;
use rand::RngExt;

use crate::{
    app::calculation,
    core::{shutdown::SHUTDOWN_REQUEST, util::datetime::Weekend},
    domain::etf::{ClassifyHint, EtfProfileRecord, EtfRepository, classify},
    infra::{
        crawler::{
            twse,
            yahoo::{self, etf::YahooEtf},
        },
        database::repository::etf::PgEtfRepository,
    },
};

mod nav_history;

pub use nav_history::{backfill_nav_history, backfill_nav_history_since, fill_recent_nav_gaps};

/// 每累積這麼多檔 Yahoo 基本資料就寫入一次，任務中途中止也不會全部白抓。
const PROFILE_BATCH: usize = 50;

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
    run_weekly_profiles(
        &PgEtfRepository::new(),
        |symbol| async move { yahoo::etf::visit(&symbol).await },
        request_interval,
        || SHUTDOWN_REQUEST.is_requested(),
    )
    .await
}

/// [`execute_weekly_profiles`] 的本體；倉儲、抓取、停頓與「是否要停」由呼叫端注入。
///
/// `stop` 回傳 `true` 時停在兩檔之間：已抓的照樣寫入並重建快照，失敗比例只以實際抓過的檔數計算。
async fn run_weekly_profiles<R, Fetch, FetchFut, Pause, PauseFut, Stop>(
    repo: &R,
    mut fetch: Fetch,
    mut pause: Pause,
    mut stop: Stop,
) -> Result<()>
where
    R: EtfRepository,
    Fetch: FnMut(String) -> FetchFut,
    FetchFut: Future<Output = Result<Option<YahooEtf>>>,
    Pause: FnMut() -> PauseFut,
    PauseFut: Future<Output = ()>,
    Stop: FnMut() -> bool,
{
    let listed = repo.listed_etfs().await?;
    let (mut saved, mut not_found, mut failed, mut unvisited) = (0usize, 0usize, 0usize, 0usize);
    let mut batch: Vec<EtfProfileRecord> = Vec::with_capacity(PROFILE_BATCH);

    for (index, (symbol, name)) in listed.iter().enumerate() {
        if stop() {
            unvisited = listed.len() - index;
            break;
        }
        if index > 0 {
            pause().await;
        }
        match fetch(symbol.clone()).await {
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
            saved += save_batch(repo, &mut batch).await?;
        }
    }
    saved += save_batch(repo, &mut batch).await?;
    let rows = repo.refresh_snapshot().await?;
    let etf_count = listed.len();
    tracing::info!(
        "ETF 基本資料更新: etfs={}, saved={saved}, not_found={not_found}, failed={failed}, unvisited={unvisited}, snapshot_rows={rows}",
        etf_count
    );
    let visited = listed.len() - unvisited;
    if too_many_failures(failed, visited) {
        bail!("Yahoo ETF 頁抓取失敗 {failed}／{visited} 檔");
    }
    Ok(())
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

async fn save_batch(repo: &impl EtfRepository, batch: &mut Vec<EtfProfileRecord>) -> Result<usize> {
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

    fn yahoo_etf(symbol: &str) -> YahooEtf {
        YahooEtf {
            profile: EtfProfileRecord {
                stock_symbol: symbol.to_string(),
                source: "yahoo".to_string(),
                ..Default::default()
            },
            portfolio: crate::domain::etf::EtfPortfolio {
                stock_symbol: symbol.to_string(),
                ..Default::default()
            },
        }
    }

    #[tokio::test]
    async fn weekly_profiles_save_found_etfs_and_rebuild_snapshot() {
        use crate::domain::etf::test_double::RecordingEtfRepository;

        let repo = RecordingEtfRepository::with_listed(&["0050", "0056", "00679B", "9999"]);
        let mut pauses = 0;
        run_weekly_profiles(
            &repo,
            |symbol| {
                let result = match symbol.as_str() {
                    "9999" => Ok(None),
                    "00679B" => Err(anyhow::anyhow!("逾時")),
                    _ => Ok(Some(yahoo_etf(&symbol))),
                };
                async move { result }
            },
            || {
                pauses += 1;
                async {}
            },
            || false,
        )
        .await
        .expect("四檔失敗一檔不算整體失敗");
        assert_eq!(pauses, 3);
        assert_eq!(repo.portfolios(), vec!["0050", "0056"]);
        assert_eq!(repo.profile_batches(), vec![vec!["0050", "0056"]]);
        assert_eq!(repo.snapshots(), 1);
    }

    #[tokio::test]
    async fn weekly_profiles_write_in_batches() {
        use crate::domain::etf::test_double::RecordingEtfRepository;

        let symbols: Vec<String> = (0..=PROFILE_BATCH).map(|i| format!("{:05}", i)).collect();
        let listed: Vec<&str> = symbols.iter().map(String::as_str).collect();
        let repo = RecordingEtfRepository::with_listed(&listed);
        run_weekly_profiles(
            &repo,
            |symbol| async move { Ok(Some(yahoo_etf(&symbol))) },
            || async {},
            || false,
        )
        .await
        .expect("全部成功");
        let sizes: Vec<usize> = repo.profile_batches().iter().map(Vec::len).collect();
        assert_eq!(sizes, vec![PROFILE_BATCH, 1], "滿一批先寫，剩下的最後寫");
    }

    #[tokio::test]
    async fn weekly_profiles_stop_between_etfs_and_keep_fetched() {
        use crate::domain::etf::test_double::RecordingEtfRepository;

        let repo = RecordingEtfRepository::with_listed(&["0050", "0056", "00679B"]);
        let mut checks = 0;
        run_weekly_profiles(
            &repo,
            |symbol| async move { Ok(Some(yahoo_etf(&symbol))) },
            || async {},
            || {
                checks += 1;
                checks > 1
            },
        )
        .await
        .expect("提早停止不算失敗");
        assert_eq!(repo.profile_batches(), vec![vec!["0050"]], "已抓的照樣寫入");
        assert_eq!(repo.snapshots(), 1);
    }

    #[tokio::test]
    async fn weekly_profiles_alert_when_most_fetches_fail() {
        use crate::domain::etf::test_double::RecordingEtfRepository;

        let repo = RecordingEtfRepository::with_listed(&["0050", "0056"]);
        let result = run_weekly_profiles(
            &repo,
            |_| async { Err(anyhow::anyhow!("封鎖")) },
            || async {},
            || false,
        )
        .await;
        let err = result.expect_err("超過一半失敗要告警");
        assert!(err.to_string().contains("2／2"), "{err}");
        assert!(repo.profile_batches().is_empty());
        assert_eq!(repo.snapshots(), 1, "失敗也照樣重建快照");
    }

    #[test]
    fn too_many_failures_means_more_than_half() {
        assert!(!too_many_failures(0, 0));
        assert!(!too_many_failures(180, 360));
        assert!(too_many_failures(181, 360));
    }
}
