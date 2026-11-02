//! ETF 淨值與單位數的讀取：週申購流量、淨值序列（優先官方淨值）與最新單位數。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn weekly_flows_sum_recent_days_with_units() {
    dotenvy::dotenv().ok();
    cleanup().await;
    // 用遠在未來的日期，確保是全表最近的幾天。
    let future = |d| NaiveDate::from_ymd_opt(2099, 1, d).expect("日期應合法");
    let repo = PgEtfRepository::new();
    let record = |date, change: i64, premium| EtfNavRecord {
        units_change: change,
        premium_rate: Some(premium),
        ..nav(date, 1_000, None)
    };
    repo.save_navs(&[
        record(future(5), 999, dec!(9.0)),
        record(future(6), 2_000, dec!(0.20)),
        record(future(7), -500, dec!(0.40)),
    ])
    .await
    .expect("寫入淨值");
    // 沒有單位數的歷史回補列不算一個交易日。
    repo.insert_nav_history(
        FAKE,
        &[EtfNavHistoryPoint {
            date: future(8),
            nav: dec!(20.0),
            market_price: None,
        }],
    )
    .await
    .expect("回補歷史");

    let calendar = trading_days(&[future(5), future(6), future(7)]).await;
    let flows = repo.weekly_flows(2).await.expect("讀資金流向");
    remove_trading_days(&calendar).await;
    let mine = flows
        .iter()
        .find(|flow| flow.stock_symbol == FAKE)
        .expect("有測試代號");
    // 預估淨值 20：(2000 − 500) × 20。
    assert_eq!(mine.net_flow, 30_000.0);
    assert_eq!((mine.from, mine.to), (future(6), future(7)));
    assert_eq!(mine.premium_last, Some(0.4));
    assert!((mine.premium_avg.expect("有平均") - 0.3).abs() < 1e-9);
    assert_eq!(mine.name, FAKE, "沒有快照時以代號當名稱");
    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn nav_series_prefers_official_nav() {
    dotenvy::dotenv().ok();
    cleanup().await;
    let repo = PgEtfRepository::new();
    repo.save_navs(&[nav(day(30), 100, None)])
        .await
        .expect("寫入 MIS 那天");
    repo.insert_nav_history(
        FAKE,
        &[EtfNavHistoryPoint {
            date: day(28),
            nav: dec!(19.50),
            market_price: None,
        }],
    )
    .await
    .expect("回補歷史");

    let series = repo
        .nav_series(&[FAKE.to_string()], day(1))
        .await
        .expect("讀淨值序列");
    let points: Vec<(NaiveDate, f64)> = series[FAKE].iter().map(|b| (b.date, b.close)).collect();
    // 4/30 還沒回填正式淨值，用預估淨值。
    assert_eq!(points, vec![(day(28), 19.5), (day(30), 20.0)]);
    assert!(
        repo.nav_series(&[FAKE.to_string()], day(30))
            .await
            .expect("起日之後")[FAKE]
            .len()
            == 1
    );
    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn latest_units_skips_rows_without_units() {
    dotenvy::dotenv().ok();
    cleanup().await;
    let repo = PgEtfRepository::new();
    repo.save_navs(&[nav(day(28), 100, None), nav(day(29), 120, None)])
        .await
        .expect("寫入 MIS");
    // 歷史回補的列沒有單位數，即使日期較新也不算。
    repo.insert_nav_history(
        FAKE,
        &[EtfNavHistoryPoint {
            date: day(30),
            nav: dec!(20.5),
            market_price: None,
        }],
    )
    .await
    .expect("回補歷史");
    let units = repo
        .latest_units(&[FAKE.to_string(), "79979XX".to_string()])
        .await
        .expect("讀單位數");
    assert_eq!(units.len(), 1);
    let point = &units[FAKE];
    assert_eq!(
        (
            point.date,
            point.outstanding_units,
            point.units_change,
            point.nav
        ),
        (day(29), 120, 1_000, Some(20.0))
    );
    cleanup().await;
}
