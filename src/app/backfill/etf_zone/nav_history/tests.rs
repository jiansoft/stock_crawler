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
