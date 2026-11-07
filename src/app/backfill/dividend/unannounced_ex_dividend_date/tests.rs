//! unannounced_ex_dividend_date的測試；連外的測試標 #[ignore]。

use rust_decimal_macros::dec;

use super::*;
use crate::infra::cache::SHARE;

fn sample_detail(
    year: i32,
    year_of_dividend: i32,
    quarter: &str,
    ex1: &str,
    ex2: &str,
    p1: &str,
    p2: &str,
) -> yahoo::dividend::YahooDividendDetail {
    yahoo::dividend::YahooDividendDetail {
        year,
        year_of_dividend,
        quarter: quarter.to_string(),
        cash_dividend: dec!(0),
        stock_dividend: dec!(0),
        ex_dividend_date1: ex1.to_string(),
        ex_dividend_date2: ex2.to_string(),
        payable_date1: p1.to_string(),
        payable_date2: p2.to_string(),
    }
}

fn sample_entity(
    year_of_dividend: i32,
    quarter: &str,
) -> crate::domain::dividend::entity::Dividend {
    crate::domain::dividend::entity::Dividend {
        serial: 0,
        security_code: "2454".to_string(),
        year: 2025,
        year_of_dividend,
        quarter: quarter.to_string(),
        earnings_cash_dividend: dec!(0),
        capital_reserve_cash_dividend: dec!(0),
        cash_dividend: dec!(0),
        earnings_stock_dividend: dec!(0),
        capital_reserve_stock_dividend: dec!(0),
        stock_dividend: dec!(0),
        sum: dec!(0),
        payout_ratio_cash: dec!(0),
        payout_ratio_stock: dec!(0),
        payout_ratio: dec!(0),
        ex_dividend_date_cash: "2025-07-01".to_string(),
        ex_dividend_date_stock: "2025-07-02".to_string(),
        payable_date_cash: "2025-08-01".to_string(),
        payable_date_stock: "2025-08-02".to_string(),
        created_time: chrono::Local::now(),
        updated_time: chrono::Local::now(),
    }
}

#[test]
fn test_find_changed_dividend_detail_when_dates_changed() {
    let entity = sample_entity(2024, "Q4");
    let details = vec![sample_detail(
        2025,
        2024,
        "Q4",
        "2025-07-10",
        "2025-07-02",
        "2025-08-01",
        "2025-08-02",
    )];

    let found = find_changed_dividend_detail(&details, &entity);
    assert!(found.is_some());
    assert_eq!(found.unwrap().ex_dividend_date1, "2025-07-10");
}

#[test]
fn test_find_changed_dividend_detail_when_dates_unchanged() {
    let entity = sample_entity(2024, "Q4");
    let details = vec![sample_detail(
        2025,
        2024,
        "Q4",
        "2025-07-01",
        "2025-07-02",
        "2025-08-01",
        "2025-08-02",
    )];

    let found = find_changed_dividend_detail(&details, &entity);
    assert!(found.is_none());
}

#[test]
fn test_find_changed_dividend_detail_when_key_not_match() {
    let entity = sample_entity(2024, "Q4");
    let details = vec![sample_detail(
        2025,
        2023,
        "Q3",
        "2025-07-10",
        "2025-07-02",
        "2025-08-01",
        "2025-08-02",
    )];

    let found = find_changed_dividend_detail(&details, &entity);
    assert!(found.is_none());
}

#[tokio::test]
#[ignore]
async fn test_backfill_unannounced_dividend_dates_live() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    let _ = backfill_unannounced_dividend_dates(2025).await;
}

/// 有待補日期的股利時，收到關機請求就在第一檔之前停下，不查 Yahoo、不寫旗標。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn backfill_until_stops_before_the_first_stock() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 backfill_until_stops_before_the_first_stock：無資料庫連接");
        return;
    }
    let symbol = "79979UD";
    let cleanup = || async {
        let _ = sqlx::query("DELETE FROM dividend WHERE security_code = $1")
            .bind(symbol)
            .execute(pool)
            .await;
    };
    cleanup().await;
    sqlx::query(r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2) VALUES ($1, 2099, 2098, '', 1, 0, 1, '-', '-', '-', '-')"#)
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入待補日期的股利");

    let result = backfill_until(2099, || true).await;
    let flag = crate::infra::nosql::redis::CLIENT
        .get_bool(&format!("yahoo:dividend:{symbol}"))
        .await
        .unwrap_or(false);
    cleanup().await;

    result.expect("提早結束不是錯誤");
    assert!(!flag, "停下時不寫旗標");
}

/// Yahoo 404 第一次只沿用 3 天略過（可能是暫時性，H37），60 天內再次 404 才拉長到 30 天。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL、Redis），請加 --features integration-tests 執行"
)]
async fn backfill_until_extends_skip_only_on_repeated_not_found() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::core::util::http::mock_origin::with_origins;
    use crate::infra::nosql::redis::CLIENT;

    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() || CLIENT.ping().await.is_err() {
        println!(
            "跳過 backfill_until_extends_skip_only_on_repeated_not_found：無資料庫或 Redis 連接"
        );
        return;
    }
    let symbol = "79979UN";
    let cache_key = format!("yahoo:dividend:{symbol}");
    let cleanup = || async {
        let _ = sqlx::query("DELETE FROM dividend WHERE security_code = $1")
            .bind(symbol)
            .execute(pool)
            .await;
        let _ = CLIENT.delete(&cache_key).await;
        let _ = CLIENT.delete(&format!("{cache_key}:not_found")).await;
    };
    cleanup().await;
    sqlx::query(r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2) VALUES ($1, 2098, 2097, '', 1, 0, 1, '尚未公布', '-', '尚未公布', '-')"#)
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入待補日期的股利");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/quote/{symbol}/dividend")))
        .respond_with(ResponseTemplate::new(404))
        .expect(2)
        .mount(&server)
        .await;

    let uri = server.uri();
    let origins = [("https://tw.stock.yahoo.com", uri.as_str())];
    let first = with_origins(&origins, backfill_until(2098, || false)).await;
    let first_ttl = CLIENT.ttl(&cache_key).await.expect("ttl");
    // 模擬 3 天略過到期，下一輪又排到這檔。
    let _ = CLIENT.delete(&cache_key).await;
    let second = with_origins(&origins, backfill_until(2098, || false)).await;
    let second_ttl = CLIENT.ttl(&cache_key).await.expect("ttl");
    cleanup().await;

    first.expect("404 只記錄，不是錯誤");
    second.expect("404 只記錄，不是錯誤");
    assert!(
        (1..=SKIP_TTL_SECONDS as i64).contains(&first_ttl),
        "第一次 404 只用 3 天略過：{first_ttl}"
    );
    assert!(
        second_ttl > SKIP_TTL_SECONDS as i64,
        "再次 404 拉長到 30 天：{second_ttl}"
    );
}
