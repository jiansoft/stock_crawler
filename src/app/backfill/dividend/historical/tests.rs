//! Yahoo 歷年股利的手動回補（單檔、指定年度所有季配/半年配股票）的測試。

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::*;
use crate::infra::{crawler::yahoo::dividend::YahooDividendDetail, database};

/// 測試用假代號；固定用歷史年度，跑完清掉。
const TEST_SYMBOL: &str = "79969";
/// 測試資料的發放年度。
const TEST_PAYOUT_YEAR: i32 = 2020;

async fn cleanup() {
    sqlx::query("DELETE FROM dividend WHERE security_code = $1")
        .bind(TEST_SYMBOL)
        .execute(database::get_connection())
        .await
        .expect("清除測試股利失敗");
}

fn quarter_detail(quarter: &str, cash_dividend: Decimal, ex_date: &str) -> YahooDividendDetail {
    YahooDividendDetail {
        year: TEST_PAYOUT_YEAR,
        year_of_dividend: TEST_PAYOUT_YEAR - 1,
        quarter: quarter.to_string(),
        cash_dividend,
        stock_dividend: Decimal::ZERO,
        ex_dividend_date1: ex_date.to_string(),
        ex_dividend_date2: "-".to_string(),
        payable_date1: "-".to_string(),
        payable_date2: "-".to_string(),
    }
}

/// 歷年明細全部寫入、季配會重算年度彙總列，且該股票會被列入季配/半年配股票清單。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn save_historical_dividends_writes_details_and_annual_total() {
    dotenvy::dotenv().ok();
    cleanup().await;

    let dividends = YahooDividend {
        stock_symbol: TEST_SYMBOL.to_string(),
        dividend: vec![(
            TEST_PAYOUT_YEAR,
            vec![
                quarter_detail("Q2", dec!(1.5), "2020-09-15"),
                quarter_detail("Q1", dec!(1.0), "2020-06-15"),
            ],
        )],
    };

    let upserted = save_historical_dividends(TEST_SYMBOL, &dividends)
        .await
        .expect("寫入歷年股利失敗");
    let annual_cash: Decimal = sqlx::query_scalar(
        "SELECT cash_dividend FROM dividend WHERE security_code = $1 AND year = $2 AND quarter = ''",
    )
    .bind(TEST_SYMBOL)
    .bind(TEST_PAYOUT_YEAR)
    .fetch_one(database::get_connection())
    .await
    .expect("讀取年度彙總列失敗");
    let symbols = multiple_dividend_symbols(TEST_PAYOUT_YEAR)
        .await
        .expect("查詢季配/半年配股票失敗");

    cleanup().await;

    assert_eq!(upserted, 2);
    assert_eq!(annual_cash, dec!(2.5));
    assert!(symbols.contains(TEST_SYMBOL));
}
