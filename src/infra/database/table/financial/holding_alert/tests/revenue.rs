//! 持股月營收通知查詢的整合測試：推估 EPS 的錨點與備援、排序、創新高與連續年減。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_holding_revenue_alerts_returns_all_holdings_sorted_by_yoy() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_fetch_holding_revenue_alerts：無資料庫連接");
        return;
    }

    cleanup().await;
    seed().await;

    let alerts = fetch_holding_revenue_alerts(TEST_DATE)
        .await
        .expect("fetch_holding_revenue_alerts");
    let fetched: Vec<_> = alerts
        .into_iter()
        .filter(|alert| fake_symbols().contains(&alert.stock_symbol))
        .collect();

    assert_eq!(fetched.len(), 4, "四檔持股都要出現，不再有年增率門檻");

    // 年增率高的排前面：B(50) > C(30) > D(20) > A(10)。
    let order: Vec<&str> = fetched
        .iter()
        .map(|alert| alert.stock_symbol.as_str())
        .collect();
    assert_eq!(
        order,
        vec![FAKE_SYMBOL_B, FAKE_SYMBOL_C, FAKE_SYMBOL_D, FAKE_SYMBOL_A]
    );

    let find = |symbol: &str| {
        fetched
            .iter()
            .find(|alert| alert.stock_symbol == symbol)
            .expect("symbol")
    };

    // A：錨點 EPS 1.2 + 1.8 ＝ 3 元，比率 1,000,000 ÷ 600,000 ⇒ 5 元。
    // 若 Q3（季底晚於本期）、全年度列或去年季度被誤納入，數字就不會是 5。
    let a = find(FAKE_SYMBOL_A);
    assert_eq!(a.stock_name, "測試甲");
    assert_eq!(a.industry_name.as_deref(), Some("水泥工業"));
    assert_eq!(a.monthly_accumulated, dec!(1000000));
    assert_eq!(a.accumulated_compared_with_last_year, dec!(12.5));
    assert_eq!(a.issued_share, 100_000_000);
    assert_eq!(a.anchor_eps, Some(dec!(3)));
    assert_eq!(a.anchor_accumulated_revenue, Some(dec!(600000)));
    let estimate = a.estimate_eps().expect("A 應該推估得出來");
    assert_eq!(estimate.basis, EpsEstimateBasis::ReportedQuarters);
    assert_eq!(estimate.accumulated, dec!(5));

    // B：查無產業、無錨點、淨利率 0（尚未回補的預設值）⇒ 不推估。
    let b = find(FAKE_SYMBOL_B);
    assert_eq!(b.industry_name, None, "查無產業分類時應為 None");
    assert_eq!(b.anchor_eps, None);
    assert_eq!(b.net_income_margin, None, "net_income = 0 的列要被排除");
    assert_eq!(b.estimate_eps(), None);

    // C：今年沒有季報 ⇒ 退回淨利率法，10 億元 × 40% ÷ 1 億股 ＝ 4 元。
    let c = find(FAKE_SYMBOL_C);
    assert_eq!(c.anchor_eps, None, "去年的季報不能當今年的錨點");
    assert_eq!(
        c.net_income_margin.map(|margin| margin.round_dp(4)),
        Some(dec!(40))
    );
    let estimate = c.estimate_eps().expect("C 應該退回淨利率法");
    assert_eq!(estimate.basis, EpsEstimateBasis::NetIncomeMargin);
    assert_eq!(estimate.accumulated, dec!(4));

    // D：缺 Q1，季別不連續 ⇒ 放棄錨點（否則會少算 Q1 而低估）。
    let d = find(FAKE_SYMBOL_D);
    assert_eq!(d.anchor_eps, None, "季別不連續時不能當錨點");
    assert_eq!(d.estimate_eps(), None);

    cleanup().await;
}

/// 創歷史新高與連續年減：A 在 1989-07～1990-07 每月 100、1990-08 為 150（高於之前 13 個月）；
/// B 只有 1990-05～08 四個月（不滿 12 個月不判斷新高），年增率 +1、−1、−2、−3（連續 3 個月年減）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_holding_revenue_alerts_marks_record_and_decline() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_fetch_holding_revenue_alerts_marks_record_and_decline：無資料庫連接");
        return;
    }
    cleanup().await;
    seed_stock(FAKE_SYMBOL_A, "測試甲", TEST_INDUSTRY, 100_000_000).await;
    seed_stock(FAKE_SYMBOL_B, "測試乙", TEST_INDUSTRY, 100_000_000).await;
    for sql in [
        r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly", "ComparedWithLastYearSameMonth")
           SELECT '79979', '79979', to_char(month, 'YYYYMM')::bigint, 100, 5
           FROM generate_series('1989-07-01'::date, '1990-07-01'::date, INTERVAL '1 month') AS month"#,
        r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly", "ComparedWithLastYearSameMonth")
           VALUES ('79979', '79979', 199008, 150, 50),
                  ('79978', '79978', 199005, 100, 1), ('79978', '79978', 199006, 100, -1),
                  ('79978', '79978', 199007, 100, -2), ('79978', '79978', 199008, 100, -3)"#,
    ] {
        sqlx::query(sql)
            .execute(database::get_connection())
            .await
            .expect("種入月營收");
    }

    let alerts = fetch_holding_revenue_alerts(199008)
        .await
        .expect("fetch_holding_revenue_alerts");
    let find = |symbol: &str| {
        alerts
            .iter()
            .find(|alert| alert.stock_symbol == symbol)
            .expect("symbol")
    };
    let a = find(FAKE_SYMBOL_A);
    assert_eq!(a.previous_high, Some(dec!(100)));
    assert!(a.is_record_high());
    assert_eq!(a.yoy_decline_months, 0);
    let b = find(FAKE_SYMBOL_B);
    assert_eq!(b.previous_high, None);
    assert!(!b.is_record_high());
    assert_eq!(b.yoy_decline_months, 3);

    cleanup().await;
}
