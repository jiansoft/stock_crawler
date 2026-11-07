//! 每日重大訊息的解析與抓取測試；連外的測試標 #[ignore]。

use super::*;

const LISTED_FIXTURE: &str = include_str!("../testdata/material_news_t187ap04_L.json");
const OTC_FIXTURE: &str = include_str!("../testdata/material_news_t187ap04_O.json");

fn at(date: (i32, u32, u32), time: (u32, u32, u32)) -> NaiveDateTime {
    NaiveDateTime::new(
        NaiveDate::from_ymd_opt(date.0, date.1, date.2).unwrap(),
        NaiveTime::from_hms_opt(time.0, time.1, time.2).unwrap(),
    )
}

/// 上市欄位：主旨欄名尾端有空白、換行接成一行、發言時間左補零。
#[test]
fn parse_listed_fixture() {
    let rows: Vec<ListedRaw> = serde_json::from_str(LISTED_FIXTURE).expect("fixture");
    let news: Vec<MaterialNews> = rows
        .into_iter()
        .filter_map(|row| {
            build(
                row.code,
                row.name,
                &row.date,
                &row.time,
                row.subject,
                row.clause,
                row.description,
            )
        })
        .collect();
    assert_eq!(news.len(), 3);
    assert_eq!(news[0].stock_symbol, "2072");
    assert_eq!(news[0].spoken_at, at((2026, 10, 5), (7, 0, 3)));
    assert_eq!(news[0].clause, "第51款");
    assert!(
        news[0]
            .subject
            .starts_with("公告本公司名稱由「世紀離岸風電設備股份有限公司」更名為")
    );
    assert!(!news[0].subject.contains('\n'));
    assert!(
        news[0]
            .description
            .starts_with("1.事實發生日：民國115年08月24日\n2.")
    );
    assert_eq!(news[2].spoken_at, at((2026, 10, 5), (11, 46, 35)));
}

/// 上櫃欄位：代號與名稱是英文欄名。
#[test]
fn parse_otc_fixture() {
    let rows: Vec<OtcRaw> = serde_json::from_str(OTC_FIXTURE).expect("fixture");
    let news: Vec<MaterialNews> = rows
        .into_iter()
        .filter_map(|row| {
            build(
                row.code,
                row.name,
                &row.date,
                &row.time,
                row.subject,
                row.clause,
                row.description,
            )
        })
        .collect();
    assert_eq!(news.len(), 2);
    assert_eq!(news[0].stock_symbol, "4530");
    assert_eq!(news[0].company_name, "天意能創");
    assert_eq!(news[0].spoken_at, at((2026, 10, 4), (7, 0, 3)));
}

#[test]
fn parse_time_pads_and_rejects_bad_values() {
    assert_eq!(parse_time("70003"), NaiveTime::from_hms_opt(7, 0, 3));
    assert_eq!(parse_time("145236"), NaiveTime::from_hms_opt(14, 52, 36));
    assert_eq!(parse_time("5"), NaiveTime::from_hms_opt(0, 0, 5));
    assert_eq!(parse_time(""), None);
    assert_eq!(parse_time("1234567"), None);
    assert_eq!(parse_time("12:00"), None);
    assert_eq!(parse_time("250000"), None);
}

/// 日期、時間或代號無法解析的列丟棄。
#[test]
fn build_drops_rows_without_a_valid_time_or_symbol() {
    let make = |code: &str, date: &str, time: &str| {
        build(
            code.to_string(),
            "名稱".to_string(),
            date,
            time,
            "主旨".to_string(),
            String::new(),
            String::new(),
        )
    };
    assert!(make("2330", "1151005", "90000").is_some());
    assert!(make(" ", "1151005", "90000").is_none());
    assert!(make("2330", "abc", "90000").is_none());
    assert!(make("2330", "1151005", "x").is_none());
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_visit() {
    dotenvy::dotenv().ok();
    let listed = visit_listed().await.expect("上市重大訊息");
    let otc = visit_otc().await.expect("上櫃重大訊息");
    dbg!(listed.len(), otc.len(), listed.first());
}

/// 以本機 mock server 取代證交所與櫃買 OpenAPI，跑完整的抓取與整理。
mod mock_sources {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;

    async fn serve(server: &MockServer, route: &str, body: &str) {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json"),
            )
            .mount(server)
            .await;
    }

    /// 上市與上櫃各自從自己的端點取得，欄位對應與 fixture 解析結果一致。
    #[tokio::test]
    async fn visit_listed_and_otc_fetch_and_build_rows() {
        let server = MockServer::start().await;
        serve(&server, "/v1/opendata/t187ap04_L", LISTED_FIXTURE).await;
        serve(&server, "/openapi/v1/mopsfin_t187ap04_O", OTC_FIXTURE).await;
        let uri = server.uri();
        let (listed, otc) = with_origins(
            &[
                ("https://openapi.twse.com.tw", &uri),
                ("https://www.tpex.org.tw", &uri),
            ],
            async { (visit_listed().await, visit_otc().await) },
        )
        .await;

        let listed = listed.expect("上市重大訊息");
        let otc = otc.expect("上櫃重大訊息");
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[0].stock_symbol, "2072");
        assert_eq!(listed[0].spoken_at, at((2026, 10, 5), (7, 0, 3)));
        assert_eq!(otc.len(), 2);
        assert_eq!(otc[0].company_name, "天意能創");
    }

    /// 時間欄位壞掉的列丟棄，其餘照常回傳。
    #[tokio::test]
    async fn visit_listed_drops_rows_with_unparsable_time() {
        let body = serde_json::json!([
            {"公司代號": "2330", "公司名稱": "台積電", "發言日期": "1151005", "發言時間": "90000",
             "主旨 ": "董事會決議", "符合條款": "第11款", "說明": "1.事實發生日"},
            {"公司代號": "2317", "公司名稱": "鴻海", "發言日期": "1151005", "發言時間": "12:00",
             "主旨 ": "時間格式錯誤", "符合條款": "", "說明": ""}
        ])
        .to_string();
        let server = MockServer::start().await;
        serve(&server, "/v1/opendata/t187ap04_L", &body).await;
        let uri = server.uri();
        let news = with_origins(&[("https://openapi.twse.com.tw", &uri)], visit_listed())
            .await
            .expect("上市重大訊息");

        assert_eq!(news.len(), 1, "{news:?}");
        assert_eq!(news[0].stock_symbol, "2330");
        assert_eq!(news[0].spoken_at, at((2026, 10, 5), (9, 0, 0)));
    }

    /// 端點不存在（404、非 JSON）時回傳錯誤，交給呼叫端決定略過。
    #[tokio::test]
    async fn visit_otc_fails_when_endpoint_is_missing() {
        let server = MockServer::start().await;
        let uri = server.uri();
        let result = with_origins(&[("https://www.tpex.org.tw", &uri)], visit_otc()).await;

        assert!(result.is_err(), "{result:?}");
    }
}
