//! 年度 EPS 補齊流程的測試：缺年報清單、Redis 略過旗標與實際抓取（連外，`#[ignore]`）。

use super::*;
use crate::domain::financial::repository::FinancialRepository;
use crate::infra::database;
use crate::infra::database::repository::financial::PgFinancialRepository;
use crate::infra::nosql::redis::CLIENT;

/// 對真實來源補齊一次（會連外並寫入資料庫）。
#[tokio::test]
#[ignore]
async fn test_execute() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 execute");

    match execute().await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("err:{:#?} ", why);
        }
    }

    tracing::debug!("結束 execute");
}

const SYMBOL: &str = "79977A";

async fn cleanup(pool: &sqlx::PgPool) {
    for sql in [
        "DELETE FROM financial_statement WHERE security_code = $1",
        "DELETE FROM stocks WHERE stock_symbol = $1",
    ] {
        let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
    }
    let _ = CLIENT
        .delete(&format!("financial_statement:annual:{SYMBOL}"))
        .await;
}

/// 缺年報的股票已有 Redis 略過旗標時不連外、不寫入，流程正常結束。
///
/// 只種一檔假代號（2024 年只有 Q1、沒有年報列）並先設旗標；資料庫裡若還有其他缺年報的股票，
/// 執行會連外抓取，因此直接略過，避免測試連外或改到真實代號的旗標。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL、Redis），請加 --features integration-tests 執行"
)]
async fn execute_skips_symbols_flagged_in_redis() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() || CLIENT.ping().await.is_err() {
        println!("跳過 execute_skips_symbols_flagged_in_redis：無資料庫或 Redis 連接");
        return;
    }
    cleanup(pool).await;
    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing") VALUES ($1, '年度 EPS 測試', $1, 2, false)"#,
    )
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("種入股票");
    sqlx::query(
        "INSERT INTO financial_statement (security_code, year, quarter, earnings_per_share) VALUES ($1, 2024, 'Q1', 1)",
    )
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("種入季報");

    let last_year = Local::now().date_naive().year() - 1;
    let missing = PgFinancialRepository::new()
        .fetch_without_annual_statements(last_year)
        .await
        .expect("查詢缺年報清單");
    let symbols: HashSet<&str> = missing
        .iter()
        .map(|statement| statement.security_code.as_str())
        .collect();
    assert!(
        symbols.contains(SYMBOL),
        "假代號應在缺年報清單：{symbols:?}"
    );
    if symbols.len() > 1 {
        println!(
            "跳過 execute_skips_symbols_flagged_in_redis：資料庫還有其他缺年報的股票 {symbols:?}"
        );
        cleanup(pool).await;
        return;
    }

    let cache_key = format!("financial_statement:annual:{SYMBOL}");
    CLIENT.set(&cache_key, true, 60).await.expect("設略過旗標");
    let result = execute().await;

    let annual: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM financial_statement WHERE security_code = $1 AND quarter = ''",
    )
    .bind(SYMBOL)
    .fetch_one(pool)
    .await
    .expect("查詢年報列");
    cleanup(pool).await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(annual, 0, "有略過旗標時不應抓取或寫入");
}

/// 以本機 mock server 取代富邦、MoneyDJ 與 MOPS（G15），驗證來源的備援順序。
mod mock_sources {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;

    const FIXTURE: &str = include_str!("../../../../infra/crawler/testdata/annual_profit.html");
    const FBS_PATH: &str = "/z/zc/zcdj_2330.djhtm";

    /// 三個來源都改送到 `server`。
    async fn fetch(server: &MockServer) -> Result<Vec<AnnualProfit>> {
        let uri = server.uri();
        with_origins(
            &[
                ("https://fubon-ebrokerdj.fbs.com.tw", &uri),
                ("https://justdata.moneydj.com", &uri),
                ("https://mopsfin.twse.com.tw", &uri),
            ],
            fetch_annual_profit("2330"),
        )
        .await
    }

    fn html(body: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(body.to_owned(), "text/html; charset=utf-8")
    }

    /// 第一個來源（富邦）有資料就直接回傳，不再問其他來源。
    #[tokio::test]
    async fn fetch_annual_profit_returns_first_source_with_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(FBS_PATH))
            .respond_with(html(FIXTURE))
            .mount(&server)
            .await;

        let profits = fetch(&server).await.expect("富邦有資料");

        assert_eq!(profits.len(), 2);
        assert_eq!((profits[0].year, profits[1].year), (2023, 2022));
        let requests = server.received_requests().await.unwrap_or_default();
        assert_eq!(requests.len(), 1, "只問富邦");
    }

    /// 富邦（同一個頁面路徑）沒有表格時改問下一個來源；MoneyDJ 也回空頁時再問 MOPS，
    /// 全部落空回錯誤。
    #[tokio::test]
    async fn fetch_annual_profit_falls_back_and_fails_when_all_sources_are_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(FBS_PATH))
            .respond_with(html("<html><body>查無資料</body></html>"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/compare/data"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let error = fetch(&server).await.expect_err("三個來源都沒有資料");

        assert!(error.to_string().contains("from all sites"), "{error:#}");
        let paths: Vec<String> = server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .map(|request| request.url.path().to_owned())
            .collect();
        assert_eq!(
            paths,
            [FBS_PATH, FBS_PATH, "/compare/data"],
            "富邦、MoneyDJ、MOPS 依序"
        );
    }
}
