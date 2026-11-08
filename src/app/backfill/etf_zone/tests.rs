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

/// 三個排程入口：週末直接結束；平日以臨時資料庫的假 ETF 跑完整流程（MIS 淨值、Yahoo 頁都導向
/// mock server：淨值抓不到時其餘步驟照做並回錯誤、Yahoo 查無頁面算查無）。假 ETF 0079979EZ，結束時刪除。
mod entries {
    use chrono::TimeZone;
    use wiremock::MockServer;

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;
    use crate::infra::database;

    const SYMBOL: &str = "0079979EZ";

    /// 1999-03-10 是星期三、03-13 是星期六。
    fn at(day: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(1999, 3, day, 17, 30, 0)
            .single()
            .expect("合法時間")
    }

    async fn cleanup() {
        let pool = database::get_connection();
        for sql in [
            "DELETE FROM etf_profile WHERE stock_symbol = $1",
            "DELETE FROM etf_snapshot WHERE stock_symbol = $1",
            "DELETE FROM etf_performance WHERE stock_symbol = $1",
            "DELETE FROM stocks WHERE stock_symbol = $1",
        ] {
            sqlx::query(sql)
                .bind(SYMBOL)
                .execute(pool)
                .await
                .expect("清除測試資料");
        }
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn entries_skip_weekends_and_run_on_weekdays() {
        dotenvy::dotenv().ok();
        let pool = database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 entries_skip_weekends_and_run_on_weekdays：無資料庫連接");
            return;
        }
        cleanup().await;
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '測試 ETF', $1, 2, 9001)"#,
        )
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("插入假 ETF");
        // 沒有掛任何路由：MIS 淨值與 Yahoo ETF 頁都回 404。
        let server = MockServer::start().await;
        let uri = server.uri();
        let origins = [
            ("https://mis.twse.com.tw", uri.as_str()),
            ("https://tw.stock.yahoo.com", uri.as_str()),
        ];

        let (weekend_daily, weekend_snapshot, weekday_daily, weekday_snapshot, weekly) =
            with_origins(&origins, async {
                (
                    run_daily(at(13)).await,
                    refresh_snapshot_on(at(13)).await,
                    run_daily(at(10)).await,
                    refresh_snapshot_on(at(10)).await,
                    execute_weekly_profiles().await,
                )
            })
            .await;
        let requests = server.received_requests().await.unwrap_or_default().len();
        let (daily_now, snapshot_now) = with_origins(&origins, async {
            (execute_daily().await, refresh_snapshot().await)
        })
        .await;
        let profile_source: Option<String> =
            sqlx::query_scalar("SELECT source FROM etf_profile WHERE stock_symbol = $1")
                .bind(SYMBOL)
                .fetch_optional(pool)
                .await
                .expect("讀回基本資料");
        cleanup().await;

        weekend_daily.expect("週末直接結束");
        weekend_snapshot.expect("週末直接結束");
        let error = weekday_daily.expect_err("淨值抓不到時回錯誤");
        assert!(error.to_string().contains("預估淨值"), "{error:#}");
        weekday_snapshot.expect("平日重建快照");
        weekly.expect("Yahoo 查無頁面只算查無");
        assert!(requests >= 2, "平日才抓 MIS 淨值與 Yahoo 頁");
        // 今天可能是週末（直接結束）或平日（淨值抓不到而回錯誤），兩種都只能是這兩個結果。
        assert!(
            daily_now
                .as_ref()
                .map_or_else(|why| why.to_string().contains("預估淨值"), |()| true),
            "{daily_now:?}"
        );
        snapshot_now.expect("重建快照");
        assert_eq!(
            profile_source.as_deref(),
            Some("heuristic"),
            "平日補上推斷的基本資料"
        );
    }
}
