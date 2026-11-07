//! revenue的測試；連外的測試標 #[ignore]。

use chrono::prelude::*;
use chrono::{Local, TimeDelta};
use std::time::Duration;

use super::*;

/// 最小 HTML fixture：兩筆有效資料列 + 標題列（應被跳過）。
///
/// 欄位順序：代號 | 名稱 | 當月 | 上月 | 去年當月 | 當月累計 | 去年累計 | 上月增減% | 去年增減% | 前期增減%
const FIXTURE_HTML: &str = r#"<html><body><table>
<tr><th>公司代號</th><th>公司名稱</th><th>當月營收</th><th>上月營收</th><th>去年當月營收</th>
    <th>當月累計</th><th>去年累計</th><th>上月增減%</th><th>去年增減%</th><th>前期增減%</th></tr>
<tr>
  <td>2330</td><td>台積電</td>
  <td>100,000,000</td><td>90,000,000</td><td>80,000,000</td>
  <td>500,000,000</td><td>450,000,000</td>
  <td>11.11</td><td>25.00</td><td>11.11</td>
</tr>
<tr>
  <td>2317</td><td>鴻海</td>
  <td>50,000,000</td><td>45,000,000</td><td>40,000,000</td>
  <td>200,000,000</td><td>180,000,000</td>
  <td>11.11</td><td>25.00</td><td>11.11</td>
</tr>
</table></body></html>"#;

#[test]
fn test_parse_revenue_html_count() {
    let result = parse_revenue_html(FIXTURE_HTML, 2026, 5);
    assert_eq!(result.len(), 2, "標題列應被跳過，只留兩筆");
}

#[test]
fn test_parse_revenue_html_date_field() {
    let result = parse_revenue_html(FIXTURE_HTML, 2026, 5);
    assert!(result.iter().all(|dto| dto.date == 202605));
}

#[test]
fn test_parse_revenue_html_symbol() {
    let result = parse_revenue_html(FIXTURE_HTML, 2026, 5);
    assert_eq!(result[0].stock_symbol, "2330");
    assert_eq!(result[1].stock_symbol, "2317");
}

#[test]
fn test_parse_revenue_html_skips_short_rows() {
    // 不足 10 欄的資料列應被跳過
    let html = r#"<html><body><table>
<tr><td>1234</td><td>公司</td><td>100</td></tr>
</table></body></html>"#;
    let result = parse_revenue_html(html, 2026, 5);
    assert!(result.is_empty());
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    let _now = Local::now();

    let naive_datetime = NaiveDate::from_ymd_opt(2026, 2, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();

    let last_month = naive_datetime - TimeDelta::try_minutes(1).unwrap();

    let timezone = FixedOffset::east_opt(8 * 60 * 60).unwrap();
    let last_month_timezone = timezone.from_local_datetime(&last_month).unwrap();
    println!("last_month_timezone:{:?}", last_month_timezone);
    match visit(last_month_timezone).await {
        Err(why) => {
            tracing::debug!("Failed to visit because: {:?}", why);
        }
        Ok(list) => {
            tracing::debug!("data:{:#?}", list);
        }
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
}

/// 以本機 mock server 取代公開資訊觀測站（C18，沿用 G15 的 mock_origin）：四個頁面
/// （上市、上櫃各兩頁）逐一下載，失敗的頁面略過、其餘照常合併。
#[tokio::test]
async fn visit_merges_pages_and_skips_failed_ones() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::core::util::http::mock_origin::with_origins;

    let server = MockServer::start().await;
    let (big5, _, _) = encoding_rs::BIG5.encode(FIXTURE_HTML);
    Mock::given(method("GET"))
        .and(path("/nas/t21/sii/t21sc03_115_5_0.html"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(big5.into_owned(), "text/html"))
        .mount(&server)
        .await;
    let date_time = FixedOffset::east_opt(8 * 3600)
        .and_then(|offset| offset.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).single())
        .expect("有效時間");

    let revenues = with_origins(
        &[("https://mopsov.twse.com.tw", &server.uri())],
        visit(date_time),
    )
    .await
    .expect("單頁失敗不影響整體");

    let symbols: Vec<&str> = revenues
        .iter()
        .map(|dto| dto.stock_symbol.as_str())
        .collect();
    assert_eq!(symbols, ["2330", "2317"]);
    let requests = server.received_requests().await.unwrap_or_default();
    assert_eq!(requests.len(), 4, "上市、上櫃各兩頁");
}
