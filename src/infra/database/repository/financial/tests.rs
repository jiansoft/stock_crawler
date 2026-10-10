//! `PgFinancialRepository` 的整合測試：以假代號走一輪寫入、讀回與彙總。

use chrono::Local;
use rust_decimal_macros::dec;

use super::*;
use crate::infra::database;

/// 假代號；測試結束會清掉所有寫入的列。
const SYMBOL: &str = "79979FR";
/// 固定的歷史年度，不使用今天的日期當資料日期。
const YEAR: i32 = 2020;

async fn cleanup() {
    let pool = database::get_connection();
    for sql in [
        "DELETE FROM financial_statement WHERE security_code = $1",
        r#"DELETE FROM "Revenue" WHERE "SecurityCode" = $1"#,
        "DELETE FROM stocks WHERE stock_symbol = $1",
    ] {
        let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
    }
}

/// 一季財報；除了指定的 EPS，其餘欄位給非零值，避免被當成「ROE／ROA 為零」。
fn statement(quarter: &str, eps: Decimal) -> DomainFinancialStatement {
    DomainFinancialStatement {
        serial: 0,
        security_code: SYMBOL.to_owned(),
        year: i64::from(YEAR),
        quarter: quarter.to_owned(),
        gross_profit: dec!(40),
        operating_profit_margin: dec!(20),
        pre_tax_income: dec!(21),
        net_income: dec!(18),
        net_asset_value_per_share: dec!(30),
        sales_per_share: dec!(12),
        earnings_per_share: eps,
        profit_before_tax: dec!(1.8),
        return_on_equity: dec!(5),
        return_on_assets: dec!(3),
        created_time: Local::now(),
        updated_time: Local::now(),
    }
}

/// 年度是否在「缺年報」清單裡。
async fn missing_annual(repo: &PgFinancialRepository) -> bool {
    repo.fetch_without_annual_statements(YEAR)
        .await
        .expect("fetch_without_annual_statements")
        .iter()
        .any(|s| s.security_code == SYMBOL && s.year == i64::from(YEAR))
}

/// Q1 的 ROE 是否被列為零。
async fn q1_has_zero_roe(repo: &PgFinancialRepository) -> bool {
    repo.fetch_roe_or_roa_equal_to_zero(Some(YEAR), Some(Quarter::Q1))
        .await
        .expect("fetch_roe_or_roa_equal_to_zero")
        .iter()
        .any(|s| s.security_code == SYMBOL)
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn financial_statements_round_trip_through_repository() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 financial_statements_round_trip_through_repository：無資料庫連接");
        return;
    }
    cleanup().await;
    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing") VALUES ($1, '財報測試', $1, 2, false)"#,
    )
    .bind(SYMBOL)
    .execute(database::get_connection())
    .await
    .expect("種入股票主檔");
    let repo = PgFinancialRepository::new();

    // Q1 單筆寫入（ROE 先是 0），Q2、Q3 批次寫入，Q4 只寫 EPS。
    let mut q1 = statement("Q1", dec!(1.5));
    q1.return_on_equity = Decimal::ZERO;
    repo.save_financial_statement(&q1).await.expect("save Q1");
    repo.batch_save_financial_statements(&[statement("Q2", dec!(2)), statement("Q3", dec!(2.5))])
        .await
        .expect("batch save Q2/Q3");
    repo.save_earnings_per_share(&statement("Q4", dec!(3)))
        .await
        .expect("save Q4 EPS");

    let cumulative = repo
        .fetch_cumulative_eps(SYMBOL, YEAR, vec![Quarter::Q1, Quarter::Q2, Quarter::Q3])
        .await
        .expect("fetch_cumulative_eps");
    assert_eq!(cumulative, dec!(6));

    // ROE 為零會被列出；更新後就不再出現。
    assert!(q1_has_zero_roe(&repo).await);
    let mut fixed = q1.clone();
    fixed.return_on_equity = dec!(4.5);
    fixed.return_on_assets = dec!(2.5);
    repo.update_statement_roe_roa(&fixed)
        .await
        .expect("update_statement_roe_roa");
    assert!(!q1_has_zero_roe(&repo).await);

    // 還沒有年報時列在缺年報清單；補寫年度 EPS 後讀得到年報、也不再缺。
    assert!(missing_annual(&repo).await);
    repo.save_annual_eps(&statement("", dec!(9)))
        .await
        .expect("save_annual_eps");
    let annual = repo
        .fetch_annual_statements(YEAR)
        .await
        .expect("fetch_annual_statements")
        .into_iter()
        .find(|s| s.security_code == SYMBOL)
        .expect("應讀到補寫的年報");
    assert_eq!(annual.earnings_per_share, dec!(9));
    assert_eq!(annual.quarter, "");
    assert!(!missing_annual(&repo).await);

    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn monthly_revenue_and_alert_queries_through_repository() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 monthly_revenue_and_alert_queries_through_repository：無資料庫連接");
        return;
    }
    cleanup().await;
    let repo = PgFinancialRepository::new();
    let revenue = DomainMonthlyRevenue {
        stock_symbol: SYMBOL.to_owned(),
        monthly: dec!(1100),
        last_month: dec!(900),
        last_year_this_month: dec!(1000),
        monthly_accumulated: dec!(3000),
        last_year_monthly_accumulated: dec!(2600),
        compared_with_last_month: dec!(22.22),
        compared_with_last_year_same_month: dec!(10),
        accumulated_compared_with_last_year: dec!(15.38),
        avg_price: dec!(100),
        lowest_price: dec!(97),
        highest_price: dec!(104),
        date: 202003,
        create_time: Local::now(),
    };
    repo.save_monthly_revenue(&revenue)
        .await
        .expect("save_monthly_revenue");
    let (monthly, highest): (Decimal, Decimal) = sqlx::query_as(
        r#"SELECT "Monthly", highest_price FROM "Revenue" WHERE "SecurityCode" = $1 AND "Date" = 202003"#,
    )
    .bind(SYMBOL)
    .fetch_one(database::get_connection())
    .await
    .expect("讀回月營收");
    assert_eq!((monthly, highest), (dec!(1100), dec!(104)));

    // 以下只依賴「今天」或整張表，確認查詢本身可執行；歷史月份的假資料不在範圍內。
    let recent = repo
        .fetch_last_two_months_revenues()
        .await
        .expect("fetch_last_two_months_revenues");
    assert!(!recent.iter().any(|r| r.stock_symbol == SYMBOL));
    repo.rebuild_revenue_last_date()
        .await
        .expect("rebuild_revenue_last_date");
    assert!(
        repo.fetch_holding_revenue_alerts(190001)
            .await
            .expect("fetch_holding_revenue_alerts")
            .is_empty()
    );
    assert!(
        repo.fetch_holding_financial_alerts(1900, "Q1")
            .await
            .expect("fetch_holding_financial_alerts")
            .is_empty()
    );

    cleanup().await;
}
