//! 台股季 EPS 更新事件的測試。

use crate::core::declare::Quarter;
use crate::infra::cache::SHARE;
use std::time::Duration;

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_execute() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 execute");

    match execute().await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("Failed to execute because {:?}", why);
        }
    }

    tracing::debug!("結束 execute");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_process_eps() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::info!("開始 process_eps");
    use crate::domain::registry::repository::StockRepository;
    let stock_repo = PgStockRepository::new();
    let without_financial_stocks = match stock_repo
        .fetch_stocks_without_financial_statement(2023, Quarter::Q4.to_string().as_str())
        .await
    {
        Ok(stocks) => stocks.into_iter().collect::<HashSet<String>>(),
        Err(why) => {
            tracing::debug!(
                "Failed to fetch stocks without financial statement: {:?}",
                why
            );
            return;
        }
    };
    //dbg!(without_financial_stocks);
    match process_eps(
        StockExchangeMarket::Listed,
        2023,
        Quarter::Q4,
        &without_financial_stocks,
    )
    .await
    {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("Failed to process_eps because: {:?}", why);
        }
    }

    tracing::info!("結束 process_eps");
    tokio::time::sleep(Duration::from_secs(1)).await;
}

/// 以本機 mock server 取代公開資訊觀測站（G15），驗證單一目標季度的抓取與寫回（G17）。
mod mock_sources {
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;
    use crate::domain::registry::entity::Stock;
    use crate::infra::database;

    const SYMBOL: &str = "79979E";
    const EPS_PATH: &str = "/mops/web/ajax_t163sb19";
    /// 固定歷史年度，避免與正式資料或其他測試的季報混在一起。
    const YEAR: i32 = 2019;

    /// 一列 9 欄的 MOPS 季 EPS 表格；`eps` 為來源的累計 EPS。
    fn eps_page(eps: &str) -> String {
        format!(
            "<html><body><table>\
             <tr><th>公司代號</th></tr>\
             <tr><td>{SYMBOL}</td><td>測試</td><td>1</td><td>{eps}</td>\
             <td>0</td><td>0</td><td>0</td><td>0</td><td>0</td></tr>\
             </table></body></html>"
        )
    }

    fn html(body: String) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(body, "text/html; charset=utf-8")
    }

    async fn cleanup(pool: &sqlx::PgPool) {
        for sql in [
            "DELETE FROM financial_statement WHERE security_code = $1",
            "DELETE FROM stocks WHERE stock_symbol = $1",
        ] {
            let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
        }
        if let Ok(mut cache) = SHARE.stocks.write() {
            cache.remove(SYMBOL);
        }
    }

    /// 上市回傳累計 EPS，扣掉已收錄的 Q1 後以單季值寫回；上櫃來源失敗只記錄，不中斷流程。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn process_target_report_saves_single_quarter_eps() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 process_target_report_saves_single_quarter_eps：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        cleanup(pool).await;
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing") VALUES ($1, '季 EPS 測試', $1, 2, false)"#,
        )
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("種入股票");
        sqlx::query(
            "INSERT INTO financial_statement (security_code, year, quarter, earnings_per_share) VALUES ($1, $2, 'Q1', 1.5)",
        )
        .bind(SYMBOL)
        .bind(YEAR)
        .execute(pool)
        .await
        .expect("種入 Q1 季報");
        // 解析時只收股票主檔快取裡有的代號。
        SHARE.stocks.write().expect("stocks 快取可寫入").insert(
            SYMBOL.to_string(),
            Stock::reconstitute(
                SYMBOL.to_string(),
                "季 EPS 測試".to_string(),
                false,
                Decimal::ZERO,
                Decimal::ZERO,
                Decimal::ZERO,
                Local::now(),
                2,
                0,
                0,
                0,
                Decimal::ZERO,
            ),
        );

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(EPS_PATH))
            .and(body_string_contains("TYPEK=sii"))
            .respond_with(html(eps_page("4.00")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(EPS_PATH))
            .and(body_string_contains("TYPEK=otc"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let uri = server.uri();
        let result = with_origins(
            &[("https://mopsov.twse.com.tw", &uri)],
            process_target_report(ReportQuarter {
                year: YEAR,
                quarter: Quarter::Q2,
            }),
        )
        .await;

        let saved: Option<Decimal> = sqlx::query_scalar(
            "SELECT earnings_per_share FROM financial_statement WHERE security_code = $1 AND year = $2 AND quarter = 'Q2'",
        )
        .bind(SYMBOL)
        .bind(YEAR)
        .fetch_optional(pool)
        .await
        .expect("查詢 Q2 季報");
        cleanup(pool).await;

        assert!(result.is_ok(), "上櫃來源失敗不應讓流程回報錯誤：{result:?}");
        assert_eq!(
            saved,
            Some(dec!(2.5)),
            "Q2 累計 4.00 扣掉 Q1 的 1.5 後應寫回單季 2.5"
        );
    }

    /// 目標季度已收錄（不在缺漏清單）的股票，即使來源有資料也不覆寫。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn process_eps_skips_symbols_already_recorded() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 process_eps_skips_symbols_already_recorded：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        cleanup(pool).await;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(EPS_PATH))
            .respond_with(html(eps_page("9.99")))
            .expect(1)
            .mount(&server)
            .await;

        let uri = server.uri();
        let result = with_origins(
            &[("https://mopsov.twse.com.tw", &uri)],
            process_eps(
                StockExchangeMarket::Listed,
                YEAR,
                Quarter::Q1,
                &HashSet::new(),
            ),
        )
        .await;

        let rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM financial_statement WHERE security_code = $1")
                .bind(SYMBOL)
                .fetch_one(pool)
                .await
                .expect("查詢季報");

        assert!(result.is_ok(), "{result:?}");
        assert_eq!(rows, 0, "不在缺漏清單的代號不應寫入");
    }
}
