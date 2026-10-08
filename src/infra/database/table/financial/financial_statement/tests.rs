//! `financial_statement` 的測試：模型與各來源轉換（純單元測試），
//! 以及寫入／查詢的整合測試（需要 PostgreSQL）。

use chrono::{Datelike, NaiveDate};
use std::time;

use rust_decimal_macros::dec;

use super::*;
use crate::{core::declare::Quarter, infra::database};

/// 寫入測試用的假代號（真實市場不存在），測試前後都會清除。
const TEST_SYMBOL: &str = "79979";
/// 寫入測試用的年度；固定用歷史年度，不會與正式資料重疊。
const TEST_YEAR: i64 = 1990;

/// 鍵值由代號、年度、季度組成，前綴版本加上型別名稱。
#[test]
fn key_combines_symbol_year_and_quarter() {
    let mut statement = FinancialStatement::new("2330".to_string());
    statement.year = 2025;
    statement.quarter = "Q3".to_string();

    assert_eq!(statement.key(), "2330-2025-Q3");
    assert_eq!(
        statement.key_with_prefix(),
        "FinancialStatement:2330-2025-Q3"
    );
}

/// `new` 只帶入代號，其他欄位都是預設值。
#[test]
fn new_statement_has_default_values() {
    let statement = FinancialStatement::new("2330".to_string());

    assert_eq!(statement.security_code, "2330");
    assert_eq!(statement.quarter, "");
    assert_eq!(statement.year, 0);
    assert_eq!(statement.serial, 0);
    assert_eq!(statement.earnings_per_share, Decimal::ZERO);
    assert_eq!(statement.return_on_equity, Decimal::ZERO);
}

/// Yahoo 與 Wespai 的季報欄位完整對應過來。
#[test]
fn quarterly_profiles_map_every_field() {
    let profile = yahoo::profile::Profile {
        quarter: "Q2".to_string(),
        stock_symbol: "2330".to_string(),
        gross_profit: dec!(58.6),
        operating_profit_margin: dec!(49.6),
        pre_tax_income: dec!(51.2),
        net_income: dec!(42.7),
        net_asset_value_per_share: dec!(201.3),
        sales_per_share: dec!(36.8),
        earnings_per_share: dec!(15.36),
        profit_before_tax: dec!(18.4),
        return_on_equity: dec!(8.1),
        return_on_assets: dec!(5.3),
        year: 2025,
    };
    let statement = FinancialStatement::from(profile);
    assert_eq!(statement.security_code, "2330");
    assert_eq!(statement.quarter, "Q2");
    assert_eq!(statement.year, 2025);
    assert_eq!(statement.gross_profit, dec!(58.6));
    assert_eq!(statement.operating_profit_margin, dec!(49.6));
    assert_eq!(statement.pre_tax_income, dec!(51.2));
    assert_eq!(statement.net_income, dec!(42.7));
    assert_eq!(statement.net_asset_value_per_share, dec!(201.3));
    assert_eq!(statement.sales_per_share, dec!(36.8));
    assert_eq!(statement.earnings_per_share, dec!(15.36));
    assert_eq!(statement.profit_before_tax, dec!(18.4));
    assert_eq!(statement.return_on_equity, dec!(8.1));
    assert_eq!(statement.return_on_assets, dec!(5.3));

    let mut profit = wespai::profit::Profit::new(2024, "2317".to_string());
    profit.quarter = "Q4".to_string();
    profit.gross_profit = dec!(6.3);
    profit.operating_profit_margin = dec!(3.1);
    profit.pre_tax_income = dec!(4.2);
    profit.net_income = dec!(2.9);
    profit.net_asset_value_per_share = dec!(117.5);
    profit.sales_per_share = dec!(492.1);
    profit.earnings_per_share = dec!(3.79);
    profit.profit_before_tax = dec!(5.0);
    profit.return_on_equity = dec!(2.6);
    profit.return_on_assets = dec!(1.0);
    let statement = FinancialStatement::from(profit);
    assert_eq!(statement.security_code, "2317");
    assert_eq!(statement.quarter, "Q4");
    assert_eq!(statement.year, 2024);
    assert_eq!(statement.gross_profit, dec!(6.3));
    assert_eq!(statement.operating_profit_margin, dec!(3.1));
    assert_eq!(statement.pre_tax_income, dec!(4.2));
    assert_eq!(statement.net_income, dec!(2.9));
    assert_eq!(statement.net_asset_value_per_share, dec!(117.5));
    assert_eq!(statement.sales_per_share, dec!(492.1));
    assert_eq!(statement.earnings_per_share, dec!(3.79));
    assert_eq!(statement.profit_before_tax, dec!(5.0));
    assert_eq!(statement.return_on_equity, dec!(2.6));
    assert_eq!(statement.return_on_assets, dec!(1.0));
}

/// 只有 EPS 的來源（證交所季 EPS、年度獲利）只帶入有的欄位，其餘保持 0。
#[test]
fn eps_only_sources_leave_other_fields_zero() {
    let eps = twse::eps::Eps::new("2330".to_string(), 2025, Quarter::Q1, dec!(13.94));
    let statement = FinancialStatement::from(eps);
    assert_eq!(statement.security_code, "2330");
    assert_eq!(statement.quarter, "Q1");
    assert_eq!(statement.year, 2025);
    assert_eq!(statement.earnings_per_share, dec!(13.94));
    assert_eq!(statement.gross_profit, Decimal::ZERO);
    assert_eq!(statement.return_on_equity, Decimal::ZERO);

    let annual = crawler::share::AnnualProfit {
        stock_symbol: "2317".to_string(),
        year: 2024,
        sales_per_share: dec!(492.1),
        earnings_per_share: dec!(11.01),
        profit_before_tax: dec!(14.2),
    };
    let statement = FinancialStatement::from(annual);
    assert_eq!(statement.security_code, "2317");
    assert_eq!(statement.quarter, "");
    assert_eq!(statement.year, 2024);
    assert_eq!(statement.sales_per_share, dec!(492.1));
    assert_eq!(statement.earnings_per_share, dec!(11.01));
    assert_eq!(statement.profit_before_tax, dec!(14.2));
    assert_eq!(statement.net_asset_value_per_share, Decimal::ZERO);
}

/// 刪除寫入測試的假資料。
async fn cleanup_test_rows() {
    sqlx::query("DELETE FROM financial_statement WHERE security_code = $1")
        .bind(TEST_SYMBOL)
        .execute(database::get_connection())
        .await
        .expect("cleanup financial_statement test rows");
}

/// 讀出假代號在指定季度的一列。
async fn fetch_test_row(quarter: &str) -> Option<FinancialStatement> {
    sqlx::query(
        r#"
SELECT serial, security_code, year, quarter, gross_profit, operating_profit_margin,
    "pre-tax_income", net_income, net_asset_value_per_share, sales_per_share,
    earnings_per_share, profit_before_tax, return_on_equity, return_on_assets,
    created_time, updated_time
FROM financial_statement
WHERE security_code = $1 AND "year" = $2 AND quarter = $3
"#,
    )
    .bind(TEST_SYMBOL)
    .bind(TEST_YEAR)
    .bind(quarter)
    .try_map(FinancialStatement::row_to_entity)
    .fetch_optional(database::get_connection())
    .await
    .expect("fetch financial_statement test row")
}

/// 組出一筆假代號的季報。
fn test_statement(quarter: &str, eps: Decimal) -> FinancialStatement {
    let mut statement = FinancialStatement::new(TEST_SYMBOL.to_string());
    statement.year = TEST_YEAR;
    statement.quarter = quarter.to_string();
    statement.earnings_per_share = eps;
    statement.gross_profit = dec!(30.5);
    statement.created_time = Local::now();
    statement.updated_time = Local::now();
    statement
}

/// 寫入函式的語意：upsert 覆寫、批次寫入、EPS 更新、年度 EPS 不覆寫、ROE／ROA 更新，
/// 以及年度查詢讀得到年度列。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn mutations_round_trip() {
    dotenvy::dotenv().ok();
    if sqlx::query("SELECT 1")
        .execute(database::get_connection())
        .await
        .is_err()
    {
        println!("跳過 mutations_round_trip：無資料庫連接");
        return;
    }
    cleanup_test_rows().await;

    // upsert：第一次新增，第二次以同鍵覆寫。
    test_statement("Q1", dec!(1.10))
        .upsert()
        .await
        .expect("upsert Q1");
    test_statement("Q1", dec!(1.25))
        .upsert()
        .await
        .expect("upsert Q1 again");
    let q1 = fetch_test_row("Q1").await.expect("Q1 row");
    assert_eq!(q1.earnings_per_share, dec!(1.25));
    assert_eq!(q1.gross_profit, dec!(30.5));

    // batch_upsert：空陣列回錯誤，多筆一次寫入。
    assert!(FinancialStatement::batch_upsert(&[]).await.is_err());
    FinancialStatement::batch_upsert(&[
        test_statement("Q2", dec!(2.00)),
        test_statement("Q3", dec!(3.00)),
    ])
    .await
    .expect("batch_upsert Q2/Q3");
    assert_eq!(
        fetch_test_row("Q3")
            .await
            .expect("Q3 row")
            .earnings_per_share,
        dec!(3.00)
    );

    // upsert_earnings_per_share：只更新 EPS。
    let mut eps_only = FinancialStatement::new(TEST_SYMBOL.to_string());
    eps_only.year = TEST_YEAR;
    eps_only.quarter = "Q2".to_string();
    eps_only.earnings_per_share = dec!(2.40);
    eps_only.created_time = Local::now();
    eps_only.updated_time = Local::now();
    eps_only
        .upsert_earnings_per_share()
        .await
        .expect("upsert_earnings_per_share");
    assert_eq!(
        fetch_test_row("Q2")
            .await
            .expect("Q2 row")
            .earnings_per_share,
        dec!(2.40)
    );

    // upsert_annual_eps：已有年度列時不覆寫。
    test_statement("", dec!(9.00))
        .upsert_annual_eps()
        .await
        .expect("upsert_annual_eps");
    test_statement("", dec!(9.99))
        .upsert_annual_eps()
        .await
        .expect("upsert_annual_eps again");
    assert_eq!(
        fetch_test_row("")
            .await
            .expect("annual row")
            .earnings_per_share,
        dec!(9.00)
    );

    // update_roe_roa：只更新報酬率。
    let mut roe = test_statement("Q1", dec!(0));
    roe.return_on_equity = dec!(12.5);
    roe.return_on_assets = dec!(6.25);
    roe.update_roe_roa().await.expect("update_roe_roa");
    let q1 = fetch_test_row("Q1").await.expect("Q1 row");
    assert_eq!(q1.return_on_equity, dec!(12.5));
    assert_eq!(q1.return_on_assets, dec!(6.25));
    assert_eq!(q1.earnings_per_share, dec!(1.25));

    // fetch_annual 讀得到年度列（quarter = ''）。
    let annual = fetch_annual(TEST_YEAR as i32)
        .await
        .expect("fetch_annual test year");
    assert!(
        annual
            .iter()
            .any(|row| row.security_code == TEST_SYMBOL && row.quarter.is_empty())
    );

    cleanup_test_rows().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_annual() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_annual");

    let r = fetch_annual(2022).await;
    if let Ok(result) = r {
        tracing::debug!("{:?}", result);
    } else if let Err(err) = r {
        tracing::debug!("{:#?} ", err);
    }
    tracing::debug!("結束 fetch_annual");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_roe_is_zero() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_roe_is_zero");

    let r = fetch_roe_or_roa_equal_to_zero(Some(2023), Some(Quarter::Q3)).await;
    if let Ok(result) = r {
        dbg!(&result);
        tracing::debug!("{:?}", result);
    } else if let Err(err) = r {
        tracing::debug!("{:#?}", err);
    }
    tracing::debug!("結束 fetch_roe_is_zero");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_without_annual() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_without_annual");

    let current_date = NaiveDate::parse_from_str("2023-09-15", "%Y-%m-%d").unwrap();
    let r = fetch_without_annual(current_date.year()).await;
    match r {
        Ok(result) => {
            //dbg!(&result);
            tracing::debug!("{:#?}", result);
        }
        Err(err) => {
            tracing::debug!("{:#?}", err);
        }
    }
    tracing::debug!("結束 fetch_without_annual");
}

#[tokio::test]
#[ignore]
async fn test_fetch_cumulative_eps() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_cumulative_eps");

    let security_code = "2480";
    let year = 2023;
    let quarters = vec![Quarter::Q1, Quarter::Q2, Quarter::Q3];
    let eps = fetch_cumulative_eps(security_code, year, quarters).await;

    match eps {
        Ok(result) => {
            dbg!(&result);
            tracing::debug!("{:#?}", result);
            // 斷言結果
            assert_eq!(result, dec!(5.51));
        }
        Err(err) => {
            tracing::debug!("{:#?}", err);
        }
    }
    tracing::debug!("結束 fetch_cumulative_eps");
    tokio::time::sleep(time::Duration::from_secs(1)).await;
}
