//! ETF 專區資料的測試。

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
