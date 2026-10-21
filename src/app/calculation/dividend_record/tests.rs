use crate::{domain::dividend::entity::Dividend, infra::cache::SHARE};
use chrono::{Local, NaiveDate, TimeZone};
use rust_decimal_macros::dec;

use super::*;

/// 驗證持有日必須嚴格早於除權息日才符合股利領取資格。
#[test]
fn test_is_holding_eligible_for_ex_date_requires_strictly_earlier_date() {
    let holding_date = NaiveDate::from_ymd_opt(2025, 6, 20).unwrap();
    let mut d = Dividend {
        serial: 0,
        year: 2025,
        year_of_dividend: 2024,
        quarter: "".to_string(),
        security_code: "".to_string(),
        earnings_cash_dividend: Decimal::ZERO,
        capital_reserve_cash_dividend: Decimal::ZERO,
        cash_dividend: Decimal::ZERO,
        earnings_stock_dividend: Decimal::ZERO,
        capital_reserve_stock_dividend: Decimal::ZERO,
        stock_dividend: Decimal::ZERO,
        sum: Decimal::ZERO,
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: "2025-06-21".to_string(),
        ex_dividend_date_stock: "2025-06-21".to_string(),
        payable_date_cash: "".to_string(),
        payable_date_stock: "".to_string(),
        created_time: Local::now(),
        updated_time: Local::now(),
    };

    assert!(d.is_eligible_for_cash(holding_date));

    d.ex_dividend_date_cash = "2025-06-20".to_string();
    assert!(!d.is_eligible_for_cash(holding_date));

    d.ex_dividend_date_cash = "2025-06-19".to_string();
    assert!(!d.is_eligible_for_cash(holding_date));

    d.ex_dividend_date_cash = "-".to_string();
    assert!(!d.is_eligible_for_cash(holding_date));
}

/// 驗證現金股利與股票股利會依各自的除息日、除權日分開判斷。
#[test]
fn test_calculate_eligible_dividend_amounts_checks_cash_and_stock_dates_separately() {
    let holding_date = NaiveDate::from_ymd_opt(2025, 6, 20).unwrap();
    let number_of_shares_held = Decimal::new(1000, 0);
    let d = Dividend {
        serial: 0,
        year: 2025,
        year_of_dividend: 2024,
        quarter: "".to_string(),
        security_code: "".to_string(),
        earnings_cash_dividend: Decimal::ZERO,
        capital_reserve_cash_dividend: Decimal::ZERO,
        cash_dividend: dec!(2.5),
        earnings_stock_dividend: Decimal::ZERO,
        capital_reserve_stock_dividend: Decimal::ZERO,
        stock_dividend: dec!(1.2),
        sum: dec!(3.7),
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: "2025-06-20".to_string(),
        ex_dividend_date_stock: "2025-06-21".to_string(),
        payable_date_cash: "".to_string(),
        payable_date_stock: "".to_string(),
        created_time: Local::now(),
        updated_time: Local::now(),
    };

    let (cash, stock, stock_money, total) = d.calculate_payout(holding_date, number_of_shares_held);

    // 持有日等於除息日，不可領現金；但仍早於除權日，所以股票股利仍可領。
    assert_eq!(cash, Decimal::ZERO);
    assert_eq!(stock_money, dec!(1200));
    assert_eq!(stock, dec!(120));
    assert_eq!(total, dec!(1200));
}

/// 驗證持有日不早於除息日與除權日時，整筆股利不會產生可領金額。
#[test]
fn test_calculate_eligible_dividend_amounts_returns_zero_when_holding_is_not_eligible() {
    let holding_date = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let number_of_shares_held = Decimal::new(1000, 0);
    let d = Dividend {
        serial: 0,
        year: 2025,
        year_of_dividend: 2024,
        quarter: "".to_string(),
        security_code: "".to_string(),
        earnings_cash_dividend: Decimal::ZERO,
        capital_reserve_cash_dividend: Decimal::ZERO,
        cash_dividend: dec!(2.5),
        earnings_stock_dividend: Decimal::ZERO,
        capital_reserve_stock_dividend: Decimal::ZERO,
        stock_dividend: dec!(1.2),
        sum: dec!(3.7),
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: "2024-06-20".to_string(),
        ex_dividend_date_stock: "2024-06-21".to_string(),
        payable_date_cash: "".to_string(),
        payable_date_stock: "".to_string(),
        created_time: Local::now(),
        updated_time: Local::now(),
    };

    let (cash, stock, stock_money, total) = d.calculate_payout(holding_date, number_of_shares_held);
    assert!(cash.is_zero());
    assert!(stock.is_zero());
    assert!(stock_money.is_zero());
    assert!(total.is_zero());
}

#[test]
fn eligible_dividend_amounts_supports_cash_only_and_stock_only_dates() {
    let holding_date = NaiveDate::from_ymd_opt(2025, 6, 20).unwrap();
    let number_of_shares_held = Decimal::new(1000, 0);
    let mut d = Dividend {
        serial: 0,
        year: 2025,
        year_of_dividend: 2024,
        quarter: "".to_string(),
        security_code: "".to_string(),
        earnings_cash_dividend: Decimal::ZERO,
        capital_reserve_cash_dividend: Decimal::ZERO,
        cash_dividend: dec!(1.5),
        earnings_stock_dividend: Decimal::ZERO,
        capital_reserve_stock_dividend: Decimal::ZERO,
        stock_dividend: dec!(0.5),
        sum: dec!(2.0),
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: "2025-06-21".to_string(),
        ex_dividend_date_stock: "-".to_string(),
        payable_date_cash: "".to_string(),
        payable_date_stock: "".to_string(),
        created_time: Local::now(),
        updated_time: Local::now(),
    };

    let (cash, _stock, stock_money, total) =
        d.calculate_payout(holding_date, number_of_shares_held);
    assert_eq!(cash, dec!(1500));
    assert_eq!(stock_money, Decimal::ZERO);
    assert_eq!(total, dec!(1500));

    d.ex_dividend_date_cash = "-".to_string();
    d.ex_dividend_date_stock = "2025-06-21".to_string();
    let (cash, stock, stock_money, total) = d.calculate_payout(holding_date, number_of_shares_held);
    assert_eq!(cash, Decimal::ZERO);
    assert_eq!(stock_money, dec!(500));
    assert_eq!(stock, dec!(50));
    assert_eq!(total, dec!(500));
}

#[test]
fn eligible_dividend_amounts_treats_invalid_or_unpublished_dates_as_ineligible() {
    let holding_date = NaiveDate::from_ymd_opt(2025, 6, 20).unwrap();
    let number_of_shares_held = Decimal::new(1000, 0);
    let d = Dividend {
        serial: 0,
        year: 2025,
        year_of_dividend: 2024,
        quarter: "".to_string(),
        security_code: "".to_string(),
        earnings_cash_dividend: Decimal::ZERO,
        capital_reserve_cash_dividend: Decimal::ZERO,
        cash_dividend: dec!(1.5),
        earnings_stock_dividend: Decimal::ZERO,
        capital_reserve_stock_dividend: Decimal::ZERO,
        stock_dividend: dec!(0.5),
        sum: dec!(2.0),
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: "".to_string(),
        ex_dividend_date_stock: "2025/06/21".to_string(),
        payable_date_cash: "".to_string(),
        payable_date_stock: "".to_string(),
        created_time: Local::now(),
        updated_time: Local::now(),
    };

    let (cash, stock, stock_money, total) = d.calculate_payout(holding_date, number_of_shares_held);
    assert!(cash.is_zero());
    assert!(stock.is_zero());
    assert!(stock_money.is_zero());
    assert!(total.is_zero());
}

#[test]
fn eligible_dividend_amounts_returns_zero_for_zero_shares() {
    let holding_date = NaiveDate::from_ymd_opt(2025, 6, 20).unwrap();
    let d = Dividend {
        serial: 0,
        year: 2025,
        year_of_dividend: 2024,
        quarter: "".to_string(),
        security_code: "".to_string(),
        earnings_cash_dividend: Decimal::ZERO,
        capital_reserve_cash_dividend: Decimal::ZERO,
        cash_dividend: dec!(1.5),
        earnings_stock_dividend: Decimal::ZERO,
        capital_reserve_stock_dividend: Decimal::ZERO,
        stock_dividend: dec!(0.5),
        sum: dec!(2.0),
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: "2025-06-21".to_string(),
        ex_dividend_date_stock: "2025-06-21".to_string(),
        payable_date_cash: "".to_string(),
        payable_date_stock: "".to_string(),
        created_time: Local::now(),
        updated_time: Local::now(),
    };

    let (cash, stock, stock_money, total) = d.calculate_payout(holding_date, Decimal::ZERO);
    assert!(cash.is_zero());
    assert!(stock.is_zero());
    assert!(stock_money.is_zero());
    assert!(total.is_zero());
}

#[tokio::test]
#[ignore]
async fn test_backfill_received_dividend_records_for_stock_backfills_after_dividend_insert() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let security_code = "0056";
    backfill_received_dividend_records_for_stock(security_code)
        .await
        .expect("backfill received dividend records by stock");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_calculate() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 calculate");
    for i in 2025..2026 {
        execute(i, None).await;
        tracing::debug!("calculate({}) 完成", i);
    }
    tracing::debug!("結束 calculate");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_calculate_dividend() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 calculate_dividend");

    let portfolio_repo = PgPortfolioRepository::new();
    let dividend_repo = PgDividendRepository::new();

    let sod = StockOwnershipDetail::reconstitute(
        102,
        "2882".to_string(),
        2,
        300,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        false,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        Local.with_ymd_and_hms(2023, 4, 9, 0, 0, 0).unwrap(),
    );

    match calculate_dividend(&portfolio_repo, &dividend_repo, sod, 2023).await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("Failed to calculate_dividend because {:?}", why);
        }
    }
    tracing::debug!("結束 calculate_dividend");
}
