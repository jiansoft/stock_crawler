use super::*;

// include_str! 的路徑相對於本檔案（mops/insider_holding/tests.rs）→ mops/testdata/。
const LISTED_FIXTURE: &str = include_str!("../testdata/insider_holding_t187ap11_L.json");
const OTC_FIXTURE: &str = include_str!("../testdata/insider_holding_t187ap11_O.json");

fn parse_fixture(fixture: &str) -> Vec<InsiderHolding> {
    parse_rows(serde_json::from_str(fixture).expect("fixture 是合法 JSON"))
}

/// 上市資料：同一人多個職稱拆成多列、數字相同；「選任時持股 」欄名尾端的空白不影響解析。
#[test]
fn parse_listed_fixture() {
    let rows = parse_fixture(LISTED_FIXTURE);
    assert_eq!(rows.len(), 7);

    let wang: Vec<_> = rows.iter().filter(|row| row.name == "王貴雲").collect();
    assert_eq!(wang.len(), 2);
    assert_eq!(wang[0].stock_symbol, "1303");
    assert_eq!(wang[0].month, NaiveDate::from_ymd_opt(2026, 8, 1).unwrap());
    assert_eq!(wang[0].title, "董事本人");
    assert_eq!(wang[0].shares, 10_723_271);
    assert_eq!(wang[0].pledged, 8_300_000);
    assert_eq!(wang[0].related_pledged, 1_224_400);
    assert_eq!(wang[1].title, "副總經理本人");
    assert_eq!(wang[1].shares, wang[0].shares);
}

/// 上櫃資料欄位相同（「選任時持股」沒有尾端空白）。
#[test]
fn parse_otc_fixture() {
    let rows = parse_fixture(OTC_FIXTURE);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].stock_symbol, "6248");
    assert_eq!(rows[0].name, "台灣鋼鐵股份有限公司");
    assert_eq!(rows[0].pledged, 3_204_000);
}

/// 股數或年月無法解析的列整列丟棄，不把壞資料當成 0。
#[test]
fn parse_rows_drops_malformed_rows() {
    let mut raw: Vec<InsiderHoldingRaw> =
        serde_json::from_str(OTC_FIXTURE).expect("fixture 是合法 JSON");
    raw[0].pledged = "N/A".to_string();
    raw[1].data_month = "115".to_string();
    assert_eq!(parse_rows(raw).len(), 2);
}

/// CSV（含 BOM、CRLF、欄名尾端空白、引號內逗號）與 JSON 解析出相同的資料列。
#[test]
fn parse_body_accepts_json_and_csv() {
    let json = parse_rows(parse_body(OTC_FIXTURE.as_bytes()).expect("JSON"));
    assert_eq!(json, parse_fixture(OTC_FIXTURE));
    let raw: Vec<InsiderHoldingRaw> = serde_json::from_str(OTC_FIXTURE).expect("fixture");
    let mut csv = String::from(
        "\u{feff}出表日期,資料年月,公司代號,公司名稱,職稱,姓名,選任時持股 ,目前持股,設質股數,內部人關係人設質股數\r\n",
    );
    for row in &raw {
        csv.push_str(&format!(
            "1150918,{},{},\"沛波, Inc.\",{},\"{}\",0,{},{},{}\r\n",
            row.data_month,
            row.code,
            row.title,
            row.name,
            row.shares,
            row.pledged,
            row.related_pledged
        ));
    }
    csv.push_str("\r\n");
    let parsed = parse_rows(parse_body(csv.as_bytes()).expect("CSV"));
    assert_eq!(parsed, json);
}

/// CSV 缺欄位、空內容與非 UTF-8 都回錯誤，不會靜默變成空資料。
#[test]
fn parse_body_rejects_unusable_responses() {
    assert!(parse_body("資料年月,公司代號\n11508,6248\n".as_bytes()).is_err());
    assert!(parse_body(b"").is_err());
    assert!(parse_body(&[0xff, 0xfe, 0x00]).is_err());
    assert!(parse_body(b"{not json").is_err());
}

/// 第一次回 503、第二次回 CSV：重試後解析成功；兩次都失敗回錯誤。
#[tokio::test]
async fn visit_retries_once_and_parses_csv() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let serve = |fail_first: usize| async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf).await;
                let n = counter.fetch_add(1, Ordering::SeqCst);
                let body = "資料年月,公司代號,職稱,姓名,目前持股,設質股數,內部人關係人設質股數
11508,6248,董事長本人,甲,100,10,0
";
                let response = if n < fail_first {
                    "HTTP/1.1 503 Service Unavailable
content-length: 0
connection: close

"
                    .to_owned()
                } else {
                    format!(
                        "HTTP/1.1 200 OK
content-type: text/csv
content-length: {}
connection: close

{body}",
                        body.len()
                    )
                };
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        (format!("http://{addr}/t187ap11_L"), hits)
    };

    let (url, hits) = serve(1).await;
    let rows = visit_with(&url, Duration::ZERO).await.expect("重試後成功");
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].shares, rows[0].pledged), (100, 10));
    assert!(hits.load(Ordering::SeqCst) >= 2);

    let (url, _) = serve(usize::MAX).await;
    assert!(visit_with(&url, Duration::ZERO).await.is_err());
}

/// 引號內的逗號、換行與跳脫的雙引號。
#[test]
fn csv_records_handles_quotes() {
    let records = csv_records("a,\"b,c\",\"d\"\"e\"\n\"x\ny\",z");
    assert_eq!(
        records,
        vec![
            vec!["a".to_string(), "b,c".to_string(), "d\"e".to_string()],
            vec!["x\ny".to_string(), "z".to_string()],
        ]
    );
}

#[test]
fn parse_month_converts_roc_year_month() {
    assert_eq!(parse_month("11508"), NaiveDate::from_ymd_opt(2026, 8, 1));
    assert_eq!(parse_month(" 9912 "), NaiveDate::from_ymd_opt(2010, 12, 1));
    assert_eq!(parse_month("11513"), None);
    assert_eq!(parse_month("8"), None);
    assert_eq!(parse_month(""), None);
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_visit() {
    dotenvy::dotenv().ok();
    let listed = visit_listed().await.expect("抓取上市董監持股");
    let otc = visit_otc().await.expect("抓取上櫃董監持股");
    dbg!(listed.len(), otc.len(), listed.first(), otc.first());
    assert!(listed.len() > 10_000);
    assert!(otc.len() > 5_000);
}
