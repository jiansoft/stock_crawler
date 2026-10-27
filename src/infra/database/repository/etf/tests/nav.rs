//! ETF nav 主題的倉儲驗證。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn save_navs_upserts_and_backfills_previous_official_nav() {
    dotenvy::dotenv().ok();
    cleanup().await;
    let calendar = trading_days(&[day(29), day(30)]).await;
    let repo = PgEtfRepository::new();

    repo.save_navs(&[nav(day(29), 100, None)])
        .await
        .expect("寫入第一天");
    // 隔天的資料帶著「前一營業日淨值」，回填 4/29 的 nav；同一批重複的鍵以最後一筆為準。
    let written = repo
        .save_navs(&[nav(day(30), 1, None), nav(day(30), 200, Some(dec!(19.95)))])
        .await
        .expect("寫入第二天");
    assert_eq!(written, 2);

    let rows: Vec<(NaiveDate, i64, Option<Decimal>)> = sqlx::query_as(
        "SELECT date, outstanding_units, nav FROM etf_daily_nav WHERE stock_symbol = $1 ORDER BY date",
    )
    .bind(FAKE)
    .fetch_all(database::get_connection())
    .await
    .expect("讀回淨值");
    assert_eq!(
        rows,
        vec![(day(29), 100, Some(dec!(19.95))), (day(30), 200, None)]
    );

    // 內容相同的重送不算寫入。
    let again = repo
        .save_navs(&[nav(day(30), 200, Some(dec!(19.95)))])
        .await
        .expect("重送");
    assert_eq!(again, 0);
    assert_eq!(repo.save_navs(&[]).await.expect("空批次"), 0);

    let premiums = repo
        .recent_premiums(&[FAKE.to_string()], 1)
        .await
        .expect("讀折溢價");
    let points = &premiums[FAKE];
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].date, day(30));
    assert!((points[0].premium_rate - 0.5).abs() < 1e-9);
    remove_trading_days(&calendar).await;
    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn missing_day_gets_its_own_official_nav_and_gap_fill_completes_it() {
    dotenvy::dotenv().ok();
    cleanup().await;
    let calendar = trading_days(&[day(27), day(28), day(29)]).await;
    let repo = PgEtfRepository::new();

    // 4/28 MIS 沒抓到；4/29 帶來的「前一營業日淨值」屬於 4/28，不可寫進 4/27。
    repo.save_navs(&[nav(day(27), 100, None)])
        .await
        .expect("寫入 4/27");
    repo.save_navs(&[nav(day(29), 100, Some(dec!(19.80)))])
        .await
        .expect("寫入 4/29");

    // 每週補缺：4/28 補上市價與折溢價；4/27 的正式淨值補上；MIS 寫入的市價不動。
    let point = |date, nav, price| EtfNavHistoryPoint {
        date,
        nav,
        market_price: Some(price),
    };
    let filled = repo
        .insert_nav_history(
            FAKE,
            &[
                point(day(27), dec!(19.70), dec!(1.00)),
                point(day(28), dec!(19.85), dec!(19.80)),
                point(day(29), dec!(19.90), dec!(1.00)),
            ],
        )
        .await
        .expect("補缺日");
    assert_eq!(filled, 3, "4/27、4/29 的正式淨值與 4/28 的市價");
    let again = repo
        .insert_nav_history(FAKE, &[point(day(28), dec!(19.85), dec!(19.80))])
        .await
        .expect("重送");
    assert_eq!(again, 0, "沒有空欄可補時不寫入");

    type Row = (
        NaiveDate,
        Option<i64>,
        Option<Decimal>,
        Option<Decimal>,
        Option<Decimal>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT date, outstanding_units, nav, market_price, premium_rate FROM etf_daily_nav WHERE stock_symbol = $1 ORDER BY date",
    )
    .bind(FAKE)
    .fetch_all(database::get_connection())
    .await
    .expect("讀回淨值");
    assert_eq!(
        rows,
        vec![
            (
                day(27),
                Some(100),
                Some(dec!(19.7000)),
                Some(dec!(20.1000)),
                Some(dec!(0.5000))
            ),
            (
                day(28),
                None,
                Some(dec!(19.8000)),
                Some(dec!(19.8000)),
                Some(dec!(0.0000))
            ),
            (
                day(29),
                Some(100),
                Some(dec!(19.9000)),
                Some(dec!(20.1000)),
                Some(dec!(0.5000))
            ),
        ]
    );
    remove_trading_days(&calendar).await;
    cleanup().await;
}

/// 台股休市、海外 ETF 照常有淨值的那天（2026-04-24 五）：台股 ETF 下一個營業日帶來的
/// 「前一營業日淨值」要回填到 4/23，海外 ETF 的要回填到它自己的 4/24。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn holiday_rows_of_other_etfs_do_not_shift_the_backfill_date() {
    dotenvy::dotenv().ok();
    cleanup().await;
    const OVERSEAS: &str = "79979OS";
    let remove_overseas = || async {
        sqlx::query("DELETE FROM etf_daily_nav WHERE stock_symbol = $1")
            .bind(OVERSEAS)
            .execute(database::get_connection())
            .await
            .expect("清除海外 ETF");
    };
    remove_overseas().await;
    let calendar = trading_days(&[day(23), day(27)]).await;
    let repo = PgEtfRepository::new();
    let overseas = |date, previous_nav| EtfNavRecord {
        stock_symbol: OVERSEAS.to_string(),
        ..nav(date, 500, previous_nav)
    };

    repo.save_navs(&[nav(day(23), 100, None), overseas(day(23), None)])
        .await
        .expect("寫入 4/23");
    repo.save_navs(&[overseas(day(24), Some(dec!(30.10)))])
        .await
        .expect("寫入休市日的海外 ETF");
    repo.save_navs(&[
        nav(day(27), 100, Some(dec!(19.95))),
        overseas(day(27), Some(dec!(30.20))),
    ])
    .await
    .expect("寫入 4/27");

    let rows: Vec<(String, NaiveDate, Option<Decimal>)> = sqlx::query_as(
        "SELECT stock_symbol::text, date, nav FROM etf_daily_nav
         WHERE stock_symbol = ANY($1) ORDER BY stock_symbol, date",
    )
    .bind([FAKE, OVERSEAS])
    .fetch_all(database::get_connection())
    .await
    .expect("讀回淨值");
    remove_trading_days(&calendar).await;
    remove_overseas().await;
    cleanup().await;
    assert_eq!(
        rows,
        vec![
            (FAKE.to_string(), day(23), Some(dec!(19.95))),
            (FAKE.to_string(), day(27), None),
            (OVERSEAS.to_string(), day(23), Some(dec!(30.10))),
            (OVERSEAS.to_string(), day(24), Some(dec!(30.20))),
            (OVERSEAS.to_string(), day(27), None),
        ]
    );
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn nav_history_fills_missing_dates_only() {
    dotenvy::dotenv().ok();
    cleanup().await;
    let repo = PgEtfRepository::new();
    repo.save_navs(&[nav(day(30), 100, None)])
        .await
        .expect("寫入 MIS 那天");

    let history = |date, nav, price| EtfNavHistoryPoint {
        date,
        nav,
        market_price: price,
    };
    let inserted = repo
        .insert_nav_history(
            FAKE,
            &[
                history(day(28), dec!(20.00), Some(dec!(20.10))),
                history(day(29), dec!(19.90), None),
                history(day(30), dec!(1.00), Some(dec!(1.00))),
            ],
        )
        .await
        .expect("回補歷史淨值");
    assert_eq!(inserted, 3, "新增 4/28、4/29，並補上 4/30 空的正式淨值");
    assert_eq!(repo.insert_nav_history(FAKE, &[]).await.expect("空批次"), 0);

    type Row = (
        NaiveDate,
        Option<i64>,
        Option<Decimal>,
        Option<Decimal>,
        Option<Decimal>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT date, outstanding_units, nav, estimated_nav, premium_rate FROM etf_daily_nav WHERE stock_symbol = $1 ORDER BY date",
    )
    .bind(FAKE)
    .fetch_all(database::get_connection())
    .await
    .expect("讀回淨值");
    assert_eq!(
        rows,
        vec![
            (day(28), None, Some(dec!(20.0000)), None, Some(dec!(0.5000))),
            (day(29), None, Some(dec!(19.9000)), None, None),
            // MIS 寫入的那天只補空的正式淨值，預估淨值與折溢價不動。
            (
                day(30),
                Some(100),
                Some(dec!(1.0000)),
                Some(dec!(20.0000)),
                Some(dec!(0.5000))
            ),
        ]
    );
    cleanup().await;
}

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
