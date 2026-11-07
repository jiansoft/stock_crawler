//! 國際證券識別碼回補的測試；連外的測試標 #[ignore]。

use crate::infra::cache::SHARE;

use super::*;

#[test]
fn registry_markets_excludes_public_offering() {
    let markets: Vec<_> = registry_markets().collect();
    assert_eq!(
        markets,
        vec![
            StockExchangeMarket::Listed,
            StockExchangeMarket::OverTheCounter,
            StockExchangeMarket::Emerging,
        ]
    );
}

fn isin(name: &str, industry: &str) -> InternationalSecuritiesIdentificationNumber {
    InternationalSecuritiesIdentificationNumber {
        stock_symbol: "79979IS".to_string(),
        name: name.to_string(),
        isin_code: String::new(),
        listing_date: String::new(),
        industry: industry.to_string(),
        cfi_code: String::new(),
        market: StockExchangeMarket::Listed,
    }
}

/// 新股票寫入、資料相同不重寫、改名再寫入；公開發行（市場別對照表沒有）不寫。假代號結束時刪除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn save_changed_writes_new_and_changed_stocks_only() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 save_changed_writes_new_and_changed_stocks_only：無資料庫連接");
        return;
    }
    SHARE.load().await;
    let cleanup = || async {
        let _ = sqlx::query("DELETE FROM stocks WHERE stock_symbol = '79979IS'")
            .execute(pool)
            .await;
    };
    cleanup().await;

    let public = InternationalSecuritiesIdentificationNumber {
        market: StockExchangeMarket::Public,
        ..isin("公開發行", "水泥工業")
    };
    let first = save_changed(&[isin("測試新股", "水泥工業"), public]).await;
    let unchanged = save_changed(&[isin("測試新股", "水泥工業")]).await;
    let renamed = save_changed(&[isin("測試改名", "水泥工業")]).await;
    let name: Option<String> =
        sqlx::query_scalar(r#"SELECT "Name" FROM stocks WHERE stock_symbol = '79979IS'"#)
            .fetch_optional(pool)
            .await
            .expect("查詢股票");
    cleanup().await;

    assert_eq!((first, unchanged, renamed), (1, 0, 1));
    assert_eq!(name.as_deref(), Some("測試改名"));
}

#[tokio::test]
#[ignore]
async fn test_execute() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 execute");

    match execute().await {
        Ok(_) => {
            tracing::debug!("完成 execute");
        }
        Err(why) => {
            tracing::debug!("Failed to execute because {:?}", why);
        }
    }

    tracing::debug!("結束 execute");
}

/// 以本機 mock server 取代 TWSE ISIN 名冊頁（Big5），以固定日期跑排程入口與單一市場處理。
mod mock_source {
    use chrono::TimeZone;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;

    const ISIN: &str = "https://isin.twse.com.tw";

    /// 1999-03-10 是星期三。
    fn wednesday() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(1999, 3, 10, 5, 20, 0)
            .single()
            .expect("合法時間")
    }

    /// 1999-03-13 是星期六。
    fn saturday() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(1999, 3, 13, 5, 20, 0)
            .single()
            .expect("合法時間")
    }

    /// 只有表頭與「股票」類別列的名冊頁；`rows` 是要放進去的資料列。
    fn page(rows: &str) -> Vec<u8> {
        let html = format!(
            "<html><body><table class=\"h4\"><tbody>\
             <tr><th>證券名稱及代號</th><th>ISIN</th><th>上市日</th><th>市場別</th><th>產業別</th><th>CFI Code</th></tr>\
             <tr><td>股票</td><td>類別</td></tr>{rows}</tbody></table></body></html>"
        );
        encoding_rs::BIG5.encode(&html).0.into_owned()
    }

    async fn mount(server: &MockServer, serial: &str, body: Vec<u8>, expected: u64) {
        Mock::given(method("GET"))
            .and(path("/isin/C_public.jsp"))
            .and(query_param("strMode", serial))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .expect(expected)
            .mount(server)
            .await;
    }

    /// 平日依序抓上市、上櫃、興櫃三份名冊（不抓公開發行）；週末一份都不抓。
    #[tokio::test]
    async fn run_fetches_registry_markets_on_weekdays_only() {
        let server = MockServer::start().await;
        for serial in ["2", "4", "5"] {
            mount(&server, serial, page(""), 1).await;
        }
        mount(&server, "1", page(""), 0).await;
        let uri = server.uri();

        with_origins(&[(ISIN, &uri)], async {
            run(saturday()).await.expect("週末略過");
            run(wednesday()).await.expect("平日完成");
        })
        .await;
        // MockServer 結束時驗證各路由的呼叫次數。
    }

    /// 排程入口以現在時間執行；平日抓到的是空名冊、週末直接略過，都不連外也不寫資料庫。
    #[tokio::test]
    async fn execute_runs_with_current_time() {
        let server = MockServer::start().await;
        for serial in ["2", "4", "5"] {
            Mock::given(method("GET"))
                .and(path("/isin/C_public.jsp"))
                .and(query_param("strMode", serial))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(page("")))
                .mount(&server)
                .await;
        }
        let uri = server.uri();

        with_origins(&[(ISIN, &uri)], execute())
            .await
            .expect("排程入口完成");
    }

    /// 名冊裡的新股票寫進股票主檔；假代號結束時刪除。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn process_market_saves_new_stock_from_listing() {
        dotenvy::dotenv().ok();
        let pool = crate::infra::database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 process_market_saves_new_stock_from_listing：無資料庫連接");
            return;
        }
        SHARE.load().await;
        let cleanup = || async {
            let _ = sqlx::query("DELETE FROM stocks WHERE stock_symbol = '79979IM'")
                .execute(pool)
                .await;
        };
        cleanup().await;

        let server = MockServer::start().await;
        let row = "<tr><td>79979IM　測試名冊</td><td>TW0079979001</td><td>1999/03/10</td>\
                   <td>上市</td><td>水泥工業</td><td>ESVUFR</td></tr>";
        mount(&server, "2", page(row), 1).await;
        let uri = server.uri();
        let result = with_origins(
            &[(ISIN, &uri)],
            process_market(wednesday(), StockExchangeMarket::Listed),
        )
        .await;
        let name: Option<String> =
            sqlx::query_scalar(r#"SELECT "Name" FROM stocks WHERE stock_symbol = '79979IM'"#)
                .fetch_optional(pool)
                .await
                .expect("查詢股票");
        cleanup().await;

        result.expect("處理上市名冊");
        assert_eq!(name.as_deref(), Some("測試名冊"));
    }
}
