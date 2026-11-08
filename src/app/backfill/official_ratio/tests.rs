//! 官方本益比與股價淨值比回補的測試。

use std::sync::Mutex;

use anyhow::anyhow;
use async_trait::async_trait;
use rust_decimal_macros::dec;

use super::*;

/// 依市場回傳固定缺值日期、記下寫入內容的假倉儲。
#[derive(Default)]
struct FakeRepo {
    listed: Vec<NaiveDate>,
    otc: Vec<NaiveDate>,
    filled: Mutex<Vec<(NaiveDate, usize)>>,
    synced: Mutex<usize>,
}

#[async_trait]
impl OfficialRatioRepository for FakeRepo {
    async fn fetch_dates_to_update(
        &self,
        market_id: i32,
        _from: NaiveDate,
        _to: NaiveDate,
        policy: PePolicy,
    ) -> Result<Vec<NaiveDate>> {
        assert_eq!(
            policy,
            if market_id == 2 {
                PePolicy::FillZeroOnly
            } else {
                PePolicy::Authoritative
            }
        );
        Ok(if market_id == 2 {
            self.listed.clone()
        } else {
            self.otc.clone()
        })
    }

    async fn fill_official_ratios(
        &self,
        date: NaiveDate,
        ratios: &[OfficialRatio],
        _policy: PePolicy,
    ) -> Result<u64> {
        self.filled.lock().unwrap().push((date, ratios.len()));
        Ok(ratios.len() as u64)
    }

    async fn sync_latest_quotes(&self) -> Result<u64> {
        *self.synced.lock().unwrap() += 1;
        Ok(3)
    }
}

fn day(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2020, 6, day).expect("有效日期")
}

fn ratio(symbol: &str) -> OfficialRatio {
    OfficialRatio {
        stock_symbol: symbol.to_owned(),
        price_earning_ratio: Some(dec!(10)),
        price_to_book_ratio: None,
    }
}

/// 兩市場各自的缺值日都抓一次；單日失敗不中斷；請求之間才停頓。
#[tokio::test]
async fn run_visits_missing_dates_and_tolerates_failures() {
    let repo = FakeRepo {
        listed: vec![day(29), day(30)],
        otc: vec![day(30)],
        ..Default::default()
    };
    let pauses = Mutex::new(0);
    let summary = run(
        &repo,
        day(1),
        day(30),
        |market, date| async move {
            match (market, date == day(29)) {
                (Market::Listed, true) => Err(anyhow!("逾時")),
                (Market::Listed, false) => Ok(vec![ratio("1101"), ratio("2330")]),
                (Market::Otc, _) => Ok(vec![ratio("6488")]),
            }
        },
        || {
            *pauses.lock().unwrap() += 1;
            async {}
        },
    )
    .await
    .expect("部分失敗不是錯誤");
    assert_eq!(
        summary,
        Summary {
            requests: 3,
            failed: 1,
            fetched_rows: 3,
            updated_rows: 3,
            synced_latest_rows: 3
        }
    );
    assert_eq!(
        *repo.synced.lock().unwrap(),
        1,
        "有更新才同步最新報價，且只同步一次"
    );
    assert_eq!(*pauses.lock().unwrap(), 2);
    assert_eq!(
        *repo.filled.lock().unwrap(),
        vec![(day(30), 2), (day(30), 1)]
    );
}

/// 全部失敗回錯誤；沒有缺值日不發請求、也不算失敗。
#[tokio::test]
async fn run_fails_only_when_every_request_fails() {
    let repo = FakeRepo {
        listed: vec![day(30)],
        ..Default::default()
    };
    let result = run(
        &repo,
        day(1),
        day(30),
        |_, _| async { Err(anyhow!("封鎖")) },
        || async {},
    )
    .await;
    assert!(result.is_err());

    let idle_repo = FakeRepo::default();
    let idle = run(
        &idle_repo,
        day(1),
        day(30),
        |_, _| async { Ok(Vec::new()) },
        || async {},
    )
    .await
    .expect("沒有缺值不是錯誤");
    assert_eq!(idle, Summary::default());
    assert_eq!(*idle_repo.synced.lock().unwrap(), 0, "沒有更新不同步");
    assert_eq!(
        (Market::Listed.market_id(), Market::Otc.market_id()),
        (2, 4)
    );
}

/// 排程回看區間含今天共 7 天。
#[test]
fn recent_window_covers_seven_days() {
    let today = NaiveDate::from_ymd_opt(2026, 10, 12).expect("有效日期");
    assert_eq!(
        recent_window(today),
        (
            NaiveDate::from_ymd_opt(2026, 10, 6).expect("有效日期"),
            today
        )
    );
}

/// 以本機 mock server 取代證交所與櫃買：真正的爬蟲接線、整段回補與排程入口。
mod mock_sources {
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;
    use crate::infra::database;

    const TWSE: &str = "https://www.twse.com.tw";
    const TPEX: &str = "https://www.tpex.org.tw";
    const TWSE_PATH: &str = "/exchangeReport/BWIBBU_d";
    const TPEX_PATH: &str = "/www/zh-tw/afterTrading/peQryDate";
    const BWIBBU_FIXTURE: &str =
        include_str!("../../../infra/crawler/twse/testdata/bwibbu_d_20200630.json");
    const PE_QRY_FIXTURE: &str =
        include_str!("../../../infra/crawler/tpex/testdata/pe_qry_date_20200630.json");

    /// 假上市股票（1999-03-10 的日線本益比、淨值比都是 0）。
    const SYMBOL: &str = "79979OR";

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("合法日期")
    }

    async fn serve(server: &MockServer, route: &str, body: &str) {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json"),
            )
            .mount(server)
            .await;
    }

    /// 依市場呼叫對應的爬蟲：上市讀 BWIBBU_d、上櫃讀 peQryDate。
    #[tokio::test]
    async fn fetch_routes_each_market_to_its_source() {
        let server = MockServer::start().await;
        serve(&server, TWSE_PATH, BWIBBU_FIXTURE).await;
        serve(&server, TPEX_PATH, PE_QRY_FIXTURE).await;
        let uri = server.uri();

        let (listed, otc) = with_origins(&[(TWSE, &uri), (TPEX, &uri)], async {
            (
                fetch(Market::Listed, date(2020, 6, 30)).await,
                fetch(Market::Otc, date(2020, 6, 30)).await,
            )
        })
        .await;

        let listed = listed.expect("上市");
        let otc = otc.expect("上櫃");
        assert!(
            listed.iter().any(|ratio| ratio.stock_symbol == "2330"),
            "{listed:?}"
        );
        assert!(!otc.is_empty());
    }

    async fn cleanup() {
        let pool = database::get_connection();
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
            "DELETE FROM stocks WHERE stock_symbol = $1",
        ] {
            sqlx::query(sql)
                .bind(SYMBOL)
                .execute(pool)
                .await
                .expect("清除測試資料");
        }
    }

    /// 上市缺值的那一天向證交所補本益比與淨值比；排程入口（最近 7 天沒有缺值）不發任何請求。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn backfill_range_fills_missing_listed_ratios() {
        dotenvy::dotenv().ok();
        let pool = database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 backfill_range_fills_missing_listed_ratios：無資料庫連接");
            return;
        }
        cleanup().await;
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id) VALUES ($1, $1, $1, 2)"#,
        )
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("插入股票");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "TradingVolume") VALUES ($1, $2, 50, 1000)"#,
        )
        .bind(date(1999, 3, 10))
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("插入日線");

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(TWSE_PATH))
            .and(query_param("date", "19990310"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "stat": "OK",
                "fields": ["證券代號", "證券名稱", "收盤價", "殖利率(%)", "股利年度", "本益比", "股價淨值比", "財報年/季"],
                "data": [[SYMBOL, "測試", "50.00", "2.00", 87, "12.34", "1.50", "88/1"]]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let uri = server.uri();

        let summary = with_origins(
            &[(TWSE, &uri), (TPEX, &uri)],
            backfill_range(date(1999, 3, 10), date(1999, 3, 10)),
        )
        .await;
        let ratios: Option<(Decimal, Decimal)> = sqlx::query_as(
            r#"SELECT "PriceEarningRatio", "price-to-book_ratio" FROM "DailyQuotes" WHERE stock_symbol = $1"#,
        )
        .bind(SYMBOL)
        .fetch_optional(pool)
        .await
        .expect("讀回日線");
        cleanup().await;

        let summary = summary.expect("回補成功");
        assert_eq!(
            (summary.requests, summary.failed, summary.updated_rows),
            (1, 0, 1)
        );
        assert_eq!(ratios, Some((dec!(12.34), dec!(1.50))));
    }

    /// 排程入口：最近 7 天沒有缺值的日線時不發請求，之後照常重算市場估值水位。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn execute_runs_recent_window() {
        dotenvy::dotenv().ok();
        if sqlx::query("SELECT 1")
            .execute(database::get_connection())
            .await
            .is_err()
        {
            println!("跳過 execute_runs_recent_window：無資料庫連接");
            return;
        }
        let server = MockServer::start().await;
        serve(
            &server,
            TWSE_PATH,
            r#"{"stat":"很抱歉，沒有符合條件的資料!"}"#,
        )
        .await;
        serve(&server, TPEX_PATH, r#"{"stat":"ok","tables":[]}"#).await;
        let uri = server.uri();

        with_origins(&[(TWSE, &uri), (TPEX, &uri)], execute())
            .await
            .expect("排程入口完成");
    }
}
