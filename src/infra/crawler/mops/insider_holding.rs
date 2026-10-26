//! # 董監事持股餘額明細（含設質）
//!
//! 公開資訊觀測站每月彙整的內部人（董事、監察人、經理人、大股東）持股與設質，
//! 由交易所開放資料提供，一次回傳全市場最新一個月份，不需要查詢參數：
//!
//! | 函式 | 來源 |
//! |------|------|
//! | [`visit_listed`] | 證交所 OpenAPI `opendata/t187ap11_L`（上市） |
//! | [`visit_otc`] | 櫃買中心 OpenAPI `mopsfin_t187ap11_O`（上櫃） |
//!
//! ## 已知限制與陷阱
//!
//! - 只有**最新一個月份**，沒有歷史；要看變動得自己保存前一期。
//! - 每月約 18 日出表（例如 2026-09-18 出 8 月資料）。
//! - 解壓後上市約 10 MiB、上櫃約 7 MiB，超過 HTTP helper 預設的 8 MiB 上限，
//!   因此用 [`util::http::get_json_with_limit`] 放寬到 [`MAX_BODY_BYTES`]。
//! - 同一人有多個職稱時（例如「董事長本人」與「總經理本人」）會拆成多列、持股數字相同。
//! - 上市的「選任時持股」欄名尾端多一個空白，本模組不用這個欄位。
//! - 證交所偶爾不回 JSON，改回同欄位的 **CSV**（UTF-8 BOM 開頭，2026-10-21 實測），因此依內容開頭判斷格式，
//!   兩種都解析；欄名一律去掉前後空白再比對。
//! - 10 MiB 在 15 秒預設逾時內常讀不完（2026-10-20、10-22 逾時），改用 [`DOWNLOAD_TIMEOUT`] 並重試一次。
//!
//! BigGo 財經也有董監持股（`/stock/major-shareholder`），但 2026-10 實測上市只到 7 月、
//! 上櫃停在 2025-11，因此改用官方來源。

use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::core::util::{self, datetime, text};

/// 上市公司董監事持股餘額明細。
const LISTED_URL: &str = "https://openapi.twse.com.tw/v1/opendata/t187ap11_L";

/// 上櫃公司董監事持股餘額明細。
const OTC_URL: &str = "https://www.tpex.org.tw/openapi/v1/mopsfin_t187ap11_O";

/// 回應 body 上限（32 MiB）；目前最大的上市資料解壓後約 10 MiB。
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// 單次下載逾時：上市資料解壓後約 10 MiB，pi 的網路常超過預設 15 秒。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(90);

/// 下載失敗後重試前的等待。
const RETRY_DELAY: Duration = Duration::from_secs(10);

/// 開放資料的原始資料列；只宣告會用到的欄位。
#[derive(Deserialize, Debug, Clone)]
struct InsiderHoldingRaw {
    /// 資料年月（民國，例如 `11508`）。
    #[serde(rename = "資料年月")]
    data_month: String,
    /// 公司代號。
    #[serde(rename = "公司代號")]
    code: String,
    /// 職稱（例如 `董事長本人`、`董事之法人代表人`）。
    #[serde(rename = "職稱")]
    title: String,
    /// 姓名（法人董事為公司名稱）。
    #[serde(rename = "姓名")]
    name: String,
    /// 目前持股（股）。
    #[serde(rename = "目前持股")]
    shares: String,
    /// 設質股數（股）。
    #[serde(rename = "設質股數")]
    pledged: String,
    /// 內部人關係人（配偶、未成年子女等）設質股數（股）。
    #[serde(rename = "內部人關係人設質股數")]
    related_pledged: String,
}

/// 單一內部人在單一職稱下的持股與設質。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsiderHolding {
    /// 股票代號。
    pub stock_symbol: String,
    /// 資料月份（該月 1 日）。
    pub month: NaiveDate,
    /// 職稱。
    pub title: String,
    /// 姓名。
    pub name: String,
    /// 目前持股（股）。
    pub shares: i64,
    /// 設質股數（股）。
    pub pledged: i64,
    /// 內部人關係人設質股數（股）。
    pub related_pledged: i64,
}

/// 取得上市公司最新一個月份的董監事持股與設質。
///
/// # Errors
///
/// HTTP 請求或 JSON 反序列化失敗時回傳錯誤。
pub async fn visit_listed() -> Result<Vec<InsiderHolding>> {
    visit(LISTED_URL).await
}

/// 取得上櫃公司最新一個月份的董監事持股與設質。
///
/// # Errors
///
/// HTTP 請求或 JSON 反序列化失敗時回傳錯誤。
pub async fn visit_otc() -> Result<Vec<InsiderHolding>> {
    visit(OTC_URL).await
}

async fn visit(url: &str) -> Result<Vec<InsiderHolding>> {
    visit_with(url, RETRY_DELAY).await
}

/// [`visit`] 的本體；重試前的等待可注入，供測試驗證重試路徑。
async fn visit_with(url: &str, retry_delay: Duration) -> Result<Vec<InsiderHolding>> {
    let body = match util::http::get_bytes_with_limit(url, MAX_BODY_BYTES, DOWNLOAD_TIMEOUT).await {
        Ok(body) => body,
        Err(why) => {
            tracing::warn!(
                "董監持股下載失敗，{} 秒後重試一次: {why:#}",
                retry_delay.as_secs()
            );
            tokio::time::sleep(retry_delay).await;
            util::http::get_bytes_with_limit(url, MAX_BODY_BYTES, DOWNLOAD_TIMEOUT).await?
        }
    };
    let rows = parse_body(&body)
        .with_context(|| format!("董監持股回應無法解析: {}", util::http::redact_url(url)))?;
    Ok(parse_rows(rows))
}

/// 依內容開頭判斷 JSON（`[`）或 CSV（表頭列），去掉 UTF-8 BOM 後解析成原始資料列。
fn parse_body(body: &[u8]) -> Result<Vec<InsiderHoldingRaw>> {
    let text = std::str::from_utf8(body).context("回應不是 UTF-8")?;
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if text.starts_with('[') {
        return Ok(serde_json::from_str(text)?);
    }
    parse_csv(text)
}

/// 解析 CSV：第一列為欄名（去掉前後空白後比對），之後每列一筆；空白列略過。
fn parse_csv(text: &str) -> Result<Vec<InsiderHoldingRaw>> {
    let mut records = csv_records(text).into_iter();
    let Some(header) = records.next() else {
        bail!("CSV 沒有表頭");
    };
    let index = |name: &str| -> Result<usize> {
        header
            .iter()
            .position(|column| column.trim() == name)
            .with_context(|| format!("CSV 缺少欄位「{name}」"))
    };
    let columns = [
        index("資料年月")?,
        index("公司代號")?,
        index("職稱")?,
        index("姓名")?,
        index("目前持股")?,
        index("設質股數")?,
        index("內部人關係人設質股數")?,
    ];
    Ok(records
        .filter(|record| record.iter().any(|field| !field.trim().is_empty()))
        .map(|record| {
            let field = |i: usize| record.get(columns[i]).cloned().unwrap_or_default();
            InsiderHoldingRaw {
                data_month: field(0),
                code: field(1),
                title: field(2),
                name: field(3),
                shares: field(4),
                pledged: field(5),
                related_pledged: field(6),
            }
        })
        .collect())
}

/// 依 RFC 4180 把 CSV 拆成列與欄：雙引號包住的欄位可含逗號與換行，`""` 代表一個雙引號；
/// 接受 `\n` 與 `\r\n` 換行。
fn csv_records(text: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (true, _) => field.push(c),
            (false, '"') if field.is_empty() => quoted = true,
            (false, ',') => record.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            (false, _) => field.push(c),
        }
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

/// 將原始資料列轉成 [`InsiderHolding`]。
///
/// 年月或股數無法解析的資料列整列丟棄並記錄警告——猜一個數字會讓上層誤判成持股或設質變動。
fn parse_rows(rows: Vec<InsiderHoldingRaw>) -> Vec<InsiderHolding> {
    rows.into_iter()
        .filter_map(|row| match parse_row(&row) {
            Some(holding) => Some(holding),
            None => {
                tracing::warn!("董監持股資料列無法解析，已略過: {row:?}");
                None
            }
        })
        .collect()
}

fn parse_row(row: &InsiderHoldingRaw) -> Option<InsiderHolding> {
    Some(InsiderHolding {
        stock_symbol: row.code.trim().to_string(),
        month: parse_month(&row.data_month)?,
        title: row.title.trim().to_string(),
        name: row.name.trim().to_string(),
        shares: text::parse_i64(&row.shares, None).ok()?,
        pledged: text::parse_i64(&row.pledged, None).ok()?,
        related_pledged: text::parse_i64(&row.related_pledged, None).ok()?,
    })
}

/// 解析民國年月（`11508` → 2026-08-01）。
fn parse_month(raw: &str) -> Option<NaiveDate> {
    let raw = raw.trim();
    let split = raw.len().checked_sub(2)?;
    let roc_year: i32 = raw.get(..split)?.parse().ok()?;
    let month: u32 = raw.get(split..)?.parse().ok()?;
    NaiveDate::from_ymd_opt(datetime::roc_year_to_gregorian_year(roc_year), month, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    // include_str! 的路徑相對於本檔案（mops/insider_holding.rs）→ mops/testdata/。
    const LISTED_FIXTURE: &str = include_str!("testdata/insider_holding_t187ap11_L.json");
    const OTC_FIXTURE: &str = include_str!("testdata/insider_holding_t187ap11_O.json");

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
}
