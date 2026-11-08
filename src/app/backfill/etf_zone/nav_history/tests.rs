//! ETF 歷史淨值回補的測試。

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
        || false,
    )
    .await
    .expect("抓取失敗不應中止");
    assert_eq!(
        tally,
        NavHistoryTally {
            inserted: 2,
            empty: 1,
            failed: 1,
            unvisited: 0,
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
        || false,
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
        || false,
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
        || false,
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
        || false,
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
        || false,
    )
    .await;
    assert!(result.is_err(), "起日不早於今天要回傳錯誤");
    assert_eq!(untouched.snapshots(), 0, "起日不合法時不抓也不重建快照");
}

#[tokio::test]
async fn backfill_symbols_stops_between_symbols_when_requested() {
    let point = EtfNavHistoryPoint {
        date: NaiveDate::from_ymd_opt(2026, 10, 21).expect("日期應合法"),
        nav: rust_decimal::Decimal::ONE,
        market_price: None,
    };
    let mut fetched: Vec<String> = Vec::new();
    let mut checks = 0;
    let tally = backfill_symbols(
        &["0050", "0056", "00679B"],
        |symbol| {
            fetched.push(symbol);
            let points = vec![point.clone()];
            async move { Ok(points) }
        },
        |_, points| async move { Ok(points.len() as u64) },
        || async {},
        || {
            checks += 1;
            checks > 1
        },
    )
    .await
    .expect("提早停止不算失敗");
    assert_eq!(fetched, vec!["0050"], "第二檔之前就停");
    assert_eq!(
        tally,
        NavHistoryTally {
            inserted: 1,
            empty: 0,
            failed: 0,
            unvisited: 2,
        }
    );
}

#[tokio::test]
async fn backfill_range_stopped_early_still_rebuilds_snapshot() {
    use crate::domain::etf::test_double::RecordingEtfRepository;

    let day = NaiveDate::from_ymd_opt(2026, 10, 21).expect("日期應合法");
    let repo = RecordingEtfRepository::with_listed(&["0050", "0056", "00679B"]);
    backfill_nav_range_with(
        &repo,
        day,
        day,
        |_| async { Err(anyhow::anyhow!("不應抓取")) },
        || async {},
        || true,
    )
    .await
    .expect("一檔都沒抓不算大量失敗");
    assert!(repo.inserted().is_empty());
    assert_eq!(repo.snapshots(), 1, "停止後照樣重建快照");
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

/// 三個入口（N 年、指定起日、近 30 天補缺日）以臨時資料庫的假 ETF 與 mock MoneyDJ 跑完整流程：
/// 抓到的淨值寫進 `etf_daily_nav`、重建快照。假 ETF 0079979NH，結束時刪除。
mod entries {
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;
    use crate::infra::database;

    const SYMBOL: &str = "0079979NH";
    /// 2026-10-01～10-21 的淨值與市價（MoneyDJ 走勢圖格式）。
    const FIXTURE: &str =
        include_str!("../../../../infra/crawler/moneydj/testdata/etf_nav_0050.txt");

    async fn cleanup() {
        let pool = database::get_connection();
        for sql in [
            "DELETE FROM etf_daily_nav WHERE stock_symbol = $1",
            "DELETE FROM etf_snapshot WHERE stock_symbol = $1",
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
    async fn entries_backfill_listed_etf_from_moneydj() {
        dotenvy::dotenv().ok();
        let pool = database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 entries_backfill_listed_etf_from_moneydj：無資料庫連接");
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
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ETF/X/xdjbcd/Basic0003BCD.xdjbcd"))
            .and(query_param("etfid", format!("{SYMBOL}.TW")))
            .respond_with(ResponseTemplate::new(200).set_body_string(FIXTURE))
            .expect(3)
            .mount(&server)
            .await;
        let uri = server.uri();

        let (years, since, gaps) = with_origins(&[("https://www.moneydj.com", &uri)], async {
            (
                backfill_nav_history(1).await,
                backfill_nav_history_since(NaiveDate::from_ymd_opt(2026, 10, 1).expect("合法日期"))
                    .await,
                fill_recent_nav_gaps().await,
            )
        })
        .await;
        let rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM etf_daily_nav WHERE stock_symbol = $1")
                .bind(SYMBOL)
                .fetch_one(pool)
                .await
                .expect("讀回淨值");
        cleanup().await;

        years.expect("近 1 年回補");
        since.expect("指定起日回補");
        gaps.expect("近 30 天補缺日");
        assert_eq!(rows, 14, "fixture 的 14 個交易日");
    }
}
