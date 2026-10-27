//! ETF snapshot 主題的倉儲驗證。

use super::super::snapshot::to_decimal;
use super::*;

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
        nav_returns: Some(EtfNavReturns {
            calc_date: day(29),
            ret_1y: Some(10.123_45),
            ret_3y: None,
            ..Default::default()
        }),
        ..Default::default()
    }])
    .await
    .expect("寫入績效");
    let nav_perf: (Option<NaiveDate>, Option<Decimal>, Option<Decimal>) = sqlx::query_as(
        "SELECT nav_calc_date, nav_ret_1y, nav_ret_3y FROM etf_performance WHERE stock_symbol = $1",
    )
    .bind(FAKE)
    .fetch_one(pool)
    .await
    .expect("讀回淨值報酬");
    assert_eq!(nav_perf, (Some(day(29)), Some(dec!(10.1235)), None));

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
