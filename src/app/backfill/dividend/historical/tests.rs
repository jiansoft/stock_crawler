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

/// 以本機 mock server 取代 Yahoo 股利頁：單檔回補寫入頁面上的全部明細；指定年度的批次回補只跑
/// 有季配／半年配的股票；查無頁面回錯誤。假代號 79968，結束時刪除。
mod mock_yahoo {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;

    const SYMBOL: &str = "79968";
    const YAHOO: &str = "https://tw.stock.yahoo.com";
    /// 2753 八方雲集的 Yahoo 股利頁（UTF-8）。
    const FIXTURE: &str =
        include_str!("../../../../infra/crawler/yahoo/testdata/dividend_2753.html");

    async fn cleanup_symbol() {
        sqlx::query("DELETE FROM dividend WHERE security_code = $1")
            .bind(SYMBOL)
            .execute(database::get_connection())
            .await
            .expect("清除測試股利");
    }

    async fn yahoo() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/quote/{SYMBOL}/dividend")))
            .respond_with(ResponseTemplate::new(200).set_body_string(FIXTURE))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn backfill_entries_fetch_yahoo_and_write_details() {
        dotenvy::dotenv().ok();
        let pool = database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 backfill_entries_fetch_yahoo_and_write_details：無資料庫連接");
            return;
        }
        cleanup_symbol().await;
        let server = yahoo().await;
        let uri = server.uri();

        let single = with_origins(
            &[(YAHOO, &uri)],
            backfill_historical_dividends_for_stock(SYMBOL),
        )
        .await;
        let written: i64 =
            sqlx::query_scalar("SELECT count(*) FROM dividend WHERE security_code = $1")
                .bind(SYMBOL)
                .fetch_one(pool)
                .await
                .expect("讀回股利");
        cleanup_symbol().await;
        // 指定年度批次：先種一筆 1999 年 Q1 季配，讓這檔進入季配／半年配清單。
        sqlx::query(
            r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2)
               VALUES ($1, 1999, 1998, 'Q1', 1, 0, 1, '1999-04-01', '-', '1999-05-01', '-')"#,
        )
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("插入季配");
        let batch = with_origins(
            &[(YAHOO, &uri)],
            backfill_historical_dividends_for_multiple_dividend_stocks(1999),
        )
        .await;
        let missing = with_origins(
            &[(YAHOO, &uri)],
            backfill_historical_dividends_for_stock("79967"),
        )
        .await;
        cleanup_symbol().await;

        let single = single.expect("單檔回補");
        assert!(single > 0, "Yahoo 頁面上有股利明細");
        assert!(written >= single as i64, "明細（與年度彙總列）寫入資料庫");
        let batch = batch.expect("批次回補");
        assert!(batch.stock_count >= 1, "{batch:?}");
        assert!(missing.is_err(), "查無頁面回錯誤");
    }
}
