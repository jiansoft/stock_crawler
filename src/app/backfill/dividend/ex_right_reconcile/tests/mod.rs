//! ex_right_reconcile 的測試。
//!
//! 子模組依被測規則分開：[`plan`]（同日、日期延後、日期未公布的比對）、[`yahoo_fill`]（從 Yahoo
//! 補缺漏與配股金額）、[`exchange_fill`]（交易所資料補 ETF 配息）；這裡放共用的資料建構函式、
//! 日期切段與缺漏日誌，以及寫入資料庫的整合測試。

mod exchange_fill;
mod plan;
mod yahoo_fill;

use chrono::Local;
use rust_decimal_macros::dec;

use super::plan::*;
use super::yahoo_fill::*;
use super::*;
use crate::core::declare::StockExchangeMarket;
use rust_decimal::Decimal;

pub(super) fn row(
    serial: i64,
    year: i32,
    quarter: &str,
    cash: Decimal,
    stock: Decimal,
    ex_cash: &str,
    ex_stock: &str,
) -> Dividend {
    Dividend {
        serial,
        year,
        year_of_dividend: year - 1,
        quarter: quarter.to_string(),
        security_code: "1109".to_string(),
        earnings_cash_dividend: Decimal::ZERO,
        capital_reserve_cash_dividend: Decimal::ZERO,
        cash_dividend: cash,
        earnings_stock_dividend: Decimal::ZERO,
        capital_reserve_stock_dividend: Decimal::ZERO,
        stock_dividend: stock,
        sum: cash + stock,
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: ex_cash.to_string(),
        ex_dividend_date_stock: ex_stock.to_string(),
        payable_date_cash: "-".to_string(),
        payable_date_stock: "-".to_string(),
        created_time: Local::now(),
        updated_time: Local::now(),
    }
}

pub(super) fn cash_event(date: &str, cash: Decimal) -> ExDividendAnnouncement {
    ExDividendAnnouncement {
        stock_symbol: "1109".to_string(),
        name: "信大".to_string(),
        ex_date: NaiveDate::parse_from_str(date, DATE_FORMAT).unwrap(),
        is_cash: true,
        is_stock: false,
        cash_dividend: Some(cash),
        stock_dividend_ratio: None,
        market: StockExchangeMarket::Listed,
    }
}

pub(super) fn existing(rows: Vec<Dividend>) -> HashMap<String, Vec<Dividend>> {
    HashMap::from([("1109".to_string(), rows)])
}

/// 測試用的交易所資料截止日。
pub(super) const AS_OF: NaiveDate = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();

#[test]
fn calendar_year_chunks_split_on_year_boundaries() {
    let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
    assert_eq!(
        calendar_year_chunks(d(2024, 11, 1), d(2026, 2, 3)),
        vec![
            (d(2024, 11, 1), d(2024, 12, 31)),
            (d(2025, 1, 1), d(2025, 12, 31)),
            (d(2026, 1, 1), d(2026, 2, 3)),
        ]
    );
    assert!(calendar_year_chunks(d(2026, 2, 3), d(2026, 1, 1)).is_empty());
}

/// 日誌只列前 20 筆，沒有缺漏時不記錄。
#[test]
fn missing_events_are_labelled_and_warned() {
    assert_eq!(
        event_label(&cash_event("2026-09-23", dec!(0.08))),
        "1109 2026-09-23"
    );
    warn_missing_events("測試", &[]);
    let many: Vec<String> = (0..25).map(|index| format!("1109 #{index}")).collect();
    warn_missing_events("測試", &many);
}

/// 刪除測試用假代號的股利資料列。
pub(super) async fn delete_test_dividends(symbol: &str) {
    sqlx::query("DELETE FROM dividend WHERE security_code = $1")
        .bind(symbol)
        .execute(crate::infra::database::get_connection())
        .await
        .expect("清除測試股利資料");
}
/// 寫入模式：改寫的資料列落地、重算年度合計，`load_existing` 依股票分組讀回。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn apply_plan_saves_updates_and_rebuilds_annual_total() {
    const SYMBOL: &str = "79979RC";
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 apply_plan_saves_updates_and_rebuilds_annual_total：無資料庫連接");
        return;
    }
    let cleanup = || async {
        sqlx::query("DELETE FROM dividend WHERE security_code = $1")
            .bind(SYMBOL)
            .execute(pool)
            .await
            .expect("清除測試股利");
    };
    cleanup().await;

    let repo = PgDividendRepository::new();
    let mut original = row(0, 1990, "H1", dec!(1.5), dec!(0), "1990-07-14", "-");
    original.security_code = SYMBOL.to_string();
    repo.save(&original).await.expect("種入股利");

    let existing = load_existing(&repo, &[1990]).await.expect("讀取股利");
    let mut saved = existing[SYMBOL][0].clone();
    assert!(saved.serial > 0, "讀回的資料列要有 serial");
    saved.ex_dividend_date_cash = "1990-08-04".to_string();
    saved.cash_dividend = dec!(1.5035);
    saved.sum = dec!(1.5035);
    let plan = ReconcilePlan {
        updates: BTreeMap::from([(saved.serial, saved)]),
        ..ReconcilePlan::default()
    };
    apply_plan(&repo, &plan).await.expect("寫入核對結果");

    let rows = &load_existing(&repo, &[1990]).await.expect("重新讀取")[SYMBOL];
    let half = rows.iter().find(|r| r.quarter == "H1").expect("期別列");
    assert_eq!(
        (half.ex_dividend_date_cash.as_str(), half.cash_dividend),
        ("1990-08-04", dec!(1.5035))
    );
    let total = rows
        .iter()
        .find(|r| r.quarter.is_empty() || r.quarter == "A")
        .expect("應重算出年度合計列");
    assert_eq!(total.cash_dividend, dec!(1.5035));

    cleanup().await;
}

/// 沒有任何異動時不寫入；空的年度清單不查詢。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn apply_plan_and_load_existing_handle_empty_input() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 apply_plan_and_load_existing_handle_empty_input：無資料庫連接");
        return;
    }
    let repo = PgDividendRepository::new();
    apply_plan(&repo, &ReconcilePlan::default())
        .await
        .expect("空計畫不應失敗");
    assert!(load_existing(&repo, &[]).await.expect("空年度").is_empty());
}
