//! ETF 倉儲的淨值回填、基本資料、持股、績效與快照測試。

use rust_decimal_macros::dec;

use super::*;
use crate::domain::etf::{EtfHoldingRecord, EtfNavHistoryPoint};

/// 測試用的假代號；寫入後一律清除。
const FAKE: &str = "79979ET";

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, d).expect("日期應合法")
}

async fn cleanup() {
    for sql in [
        "DELETE FROM etf_daily_nav WHERE stock_symbol = $1",
        "DELETE FROM etf_profile WHERE stock_symbol = $1",
        "DELETE FROM etf_holding WHERE stock_symbol = $1",
        "DELETE FROM etf_allocation WHERE stock_symbol = $1",
        "DELETE FROM etf_performance WHERE stock_symbol = $1",
        "DELETE FROM etf_snapshot WHERE stock_symbol = $1",
        "DELETE FROM last_daily_quotes WHERE stock_symbol = $1",
        "DELETE FROM dividend WHERE security_code = $1",
        "DELETE FROM stocks WHERE stock_symbol = $1",
    ] {
        sqlx::query(sql)
            .bind(FAKE)
            .execute(database::get_connection())
            .await
            .expect("清除測試資料");
    }
}

fn nav(date: NaiveDate, units: i64, previous_nav: Option<Decimal>) -> EtfNavRecord {
    EtfNavRecord {
        date,
        stock_symbol: FAKE.to_string(),
        outstanding_units: units,
        units_change: 1_000,
        market_price: Some(dec!(20.10)),
        estimated_nav: Some(dec!(20.00)),
        premium_rate: Some(dec!(0.50)),
        previous_nav,
        source_time: None,
    }
}

#[test]
fn to_decimal_rounds_and_rejects_non_finite() {
    assert_eq!(to_decimal(Some(1.234_567)), Some(dec!(1.2346)));
    assert_eq!(to_decimal(Some(f64::NAN)), None);
    assert_eq!(to_decimal(None), None);
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn save_navs_upserts_and_backfills_previous_official_nav() {
    dotenvy::dotenv().ok();
    cleanup().await;
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
    cleanup().await;
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
    assert_eq!(inserted, 2);
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
            // MIS 寫入的那天不被回補覆蓋。
            (
                day(30),
                Some(100),
                None,
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
async fn heuristic_profiles_never_overwrite_source_profiles() {
    dotenvy::dotenv().ok();
    cleanup().await;
    let repo = PgEtfRepository::new();
    let heuristic = EtfProfileRecord {
        stock_symbol: FAKE.to_string(),
        strategy: "thematic".to_string(),
        region: "台灣".to_string(),
        source: "heuristic".to_string(),
        ..Default::default()
    };
    assert_eq!(
        repo.insert_missing_profiles(std::slice::from_ref(&heuristic))
            .await
            .expect("補推斷分類"),
        1
    );
    // 推斷規則改變時，推斷列跟著更新。
    let updated = EtfProfileRecord {
        strategy: "high_dividend".to_string(),
        ..heuristic.clone()
    };
    assert_eq!(
        repo.insert_missing_profiles(std::slice::from_ref(&updated))
            .await
            .expect("更新推斷分類"),
        1
    );

    let yahoo = EtfProfileRecord {
        stock_symbol: FAKE.to_string(),
        full_name: "測試基金".to_string(),
        strategy: "market_cap".to_string(),
        themes: vec!["半導體".to_string()],
        nav_return: Some(serde_json::json!({"1y": 12.5})),
        total_assets: Some(dec!(1000)),
        source: "yahoo".to_string(),
        ..Default::default()
    };
    assert_eq!(
        repo.save_profiles(std::slice::from_ref(&yahoo))
            .await
            .expect("寫入來源資料"),
        1
    );
    assert_eq!(
        repo.insert_missing_profiles(&[heuristic])
            .await
            .expect("推斷不覆蓋來源"),
        0
    );

    let (strategy, themes, source): (String, Vec<String>, String) =
        sqlx::query_as("SELECT strategy, themes, source FROM etf_profile WHERE stock_symbol = $1")
            .bind(FAKE)
            .fetch_one(database::get_connection())
            .await
            .expect("讀回基本資料");
    assert_eq!(strategy, "market_cap");
    assert_eq!(themes, vec!["半導體".to_string()]);
    assert_eq!(source, "yahoo");
    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn replace_portfolio_keeps_sections_the_source_did_not_return() {
    dotenvy::dotenv().ok();
    cleanup().await;
    let repo = PgEtfRepository::new();
    let allocation = |label: &str, weight: Decimal| EtfAllocationRecord {
        label: label.to_string(),
        weight,
    };
    let full = EtfPortfolio {
        stock_symbol: FAKE.to_string(),
        holdings_as_of: Some(day(1)),
        holdings: vec![EtfHoldingRecord {
            rank: 1,
            holding_symbol: "2330".to_string(),
            holding_name: "台積電".to_string(),
            weight: dec!(50),
        }],
        industries_as_of: Some(day(17)),
        // 同名的列只留第一筆。
        industries: vec![
            allocation("半導體業", dec!(60)),
            allocation("半導體業", dec!(1)),
            allocation("金融保險", dec!(10)),
        ],
        assets_as_of: Some(day(17)),
        assets: vec![allocation("股票", dec!(99))],
    };
    assert_eq!(repo.replace_portfolio(&full).await.expect("寫入持股"), 4);

    // 新一期只有資產分佈：前十大與行業比重保留舊資料。
    let partial = EtfPortfolio {
        stock_symbol: FAKE.to_string(),
        assets_as_of: Some(day(24)),
        assets: vec![allocation("股票", dec!(98)), allocation("現金", dec!(2))],
        ..Default::default()
    };
    assert_eq!(repo.replace_portfolio(&partial).await.expect("部分更新"), 2);

    let count = |sql: &'static str| async move {
        let (n,): (i64,) = sqlx::query_as(sql)
            .bind(FAKE)
            .fetch_one(database::get_connection())
            .await
            .expect("計數");
        n
    };
    assert_eq!(
        count("SELECT COUNT(*) FROM etf_holding WHERE stock_symbol = $1").await,
        1
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM etf_allocation WHERE stock_symbol = $1 AND kind = 'industry'")
            .await,
        2
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM etf_allocation WHERE stock_symbol = $1 AND kind = 'asset'")
            .await,
        2
    );
    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn snapshot_combines_quote_nav_dividends_and_performance() {
    dotenvy::dotenv().ok();
    cleanup().await;
    let pool = database::get_connection();
    let repo = PgEtfRepository::new();

    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id, "SuspendListing")
           VALUES ($1, '測試高股息', $1, 2, 9001, false)"#,
    )
    .bind(FAKE)
    .execute(pool)
    .await
    .expect("建立假 ETF");
    sqlx::query(
        "INSERT INTO last_daily_quotes (date, stock_symbol, closing_price, change, change_range, trading_volume, trade_value)
         VALUES ($2, $1, 20, 0.2, 1.01, 1234567, 24680000)",
    )
    .bind(FAKE)
    .bind(day(30))
    .execute(pool)
    .await
    .expect("建立假行情");
    for (quarter, cash, ex_date) in [
        ("Q4", dec!(0.5), "2025-12-01"),
        ("Q1", dec!(0.6), "2026-03-01"),
        ("Q2", dec!(0.7), "2026-06-01"),
        ("Q3", dec!(0.1), "-"),
    ] {
        sqlx::query(
            r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, "ex-dividend_date1")
               VALUES ($1, 2026, 2026, $2, $3, $4)"#,
        )
        .bind(FAKE)
        .bind(quarter)
        .bind(cash)
        .bind(ex_date)
        .execute(pool)
        .await
        .expect("建立假股利");
    }
    repo.save_navs(&[nav(day(30), 1_000_000, None)])
        .await
        .expect("寫入淨值");
    repo.insert_missing_profiles(&[EtfProfileRecord {
        stock_symbol: FAKE.to_string(),
        strategy: "high_dividend".to_string(),
        region: "台灣".to_string(),
        source: "heuristic".to_string(),
        ..Default::default()
    }])
    .await
    .expect("寫入分類");
    sqlx::query(
        r#"UPDATE etf_profile SET manual_override = '{"themes": ["金融"]}' WHERE stock_symbol = $1"#,
    )
    .bind(FAKE)
    .execute(pool)
    .await
    .expect("人工覆寫題材");
    repo.save_performances(&[EtfPerformance {
        stock_symbol: FAKE.to_string(),
        calc_date: Some(day(30)),
        ret_1y: Some(12.345_67),
        year_low: Some(18.0),
        ..Default::default()
    }])
    .await
    .expect("寫入績效");

    assert!(repo.refresh_snapshot().await.expect("重建快照") >= 1);

    type Row = (
        String,
        Vec<String>,
        String,
        Option<i64>,
        Option<Decimal>,
        Option<Decimal>,
        Option<Decimal>,
        Option<i32>,
        Option<Decimal>,
        Option<NaiveDate>,
        Option<Decimal>,
        Option<Decimal>,
        Option<Decimal>,
    );
    let row: Row = sqlx::query_as(
        "SELECT strategy, themes, dividend_frequency, trading_volume, total_assets, net_flow,
                dividend_12m, dividend_count_12m, yield_12m, next_ex_date, ret_1y, premium_avg_20d,
                last_dividend
         FROM etf_snapshot WHERE stock_symbol = $1",
    )
    .bind(FAKE)
    .fetch_one(pool)
    .await
    .expect("讀回快照");
    assert_eq!(row.0, "high_dividend");
    assert_eq!(row.1, vec!["金融".to_string()]);
    assert_eq!(row.2, "semi_annual");
    assert_eq!(row.3, Some(1235));
    assert_eq!(row.4, Some(dec!(20000000.00)));
    assert_eq!(row.5, Some(dec!(20000.00)));
    assert_eq!(row.6, Some(dec!(1.1)));
    assert_eq!(row.7, Some(2));
    assert_eq!(row.8, Some(dec!(5.5)));
    assert_eq!(row.9, NaiveDate::from_ymd_opt(2026, 6, 1));
    assert_eq!(row.10, Some(dec!(12.3457)));
    assert_eq!(row.11, Some(dec!(0.5)));
    assert_eq!(row.12, Some(dec!(0.6)));

    // 下市後從快照與績效移除。
    sqlx::query(r#"UPDATE stocks SET "SuspendListing" = true WHERE stock_symbol = $1"#)
        .bind(FAKE)
        .execute(pool)
        .await
        .expect("標記下市");
    repo.refresh_snapshot().await.expect("重建快照");
    repo.save_performances(&[]).await.expect("清除下市績效");
    let (left,): (i64,) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM etf_snapshot WHERE stock_symbol = $1)
              + (SELECT COUNT(*) FROM etf_performance WHERE stock_symbol = $1)",
    )
    .bind(FAKE)
    .fetch_one(pool)
    .await
    .expect("計數");
    assert_eq!(left, 0);
    cleanup().await;
}
