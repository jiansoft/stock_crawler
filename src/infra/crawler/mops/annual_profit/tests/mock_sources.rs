//! 以本機 mock server 取代 MOPS（C18，沿用 G15 的 mock_origin）：EPS、營收、股本三條趨勢資料
//! 加上逐年的綜合損益表，組出年度財報。

use rust_decimal_macros::dec;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::core::util::http::mock_origin::with_origins;

/// `compare/data` 的回應：2023、2024 兩年各一個值。
fn series(values: [f64; 2]) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "xaxisList": ["2023Q4", "2024Q4"],
        "graphData": [{ "data": [[0, values[0], "A"], [1, values[1], "A"]] }]
    }))
}

/// 2024 年的綜合損益表：稅前淨利 1,000,000 仟元、母公司業主淨利 800,000 仟元。
const REPORT_2024: &str = r#"
    <div id="headTable"><table><tbody>
        <tr><td>稅前淨利（淨損）</td></tr>
        <tr><td>本期淨利（淨損）</td></tr>
        <tr><td>母公司業主（淨利／損）</td></tr>
    </tbody></table></div>
    <div id="bodyTable"><table><tbody>
        <tr><td>1,000,000</td></tr>
        <tr><td>820,000</td></tr>
        <tr><td>800,000</td></tr>
    </tbody></table></div>
"#;

async fn mount_series(server: &MockServer, item: &str, values: [f64; 2]) {
    Mock::given(method("POST"))
        .and(path("/compare/data"))
        .and(body_string_contains(format!("compareItem={item}")))
        .respond_with(series(values))
        .mount(server)
        .await;
}

async fn visit_mocked(server: &MockServer) -> Result<Vec<share::AnnualProfit>> {
    with_origins(
        &[("https://mopsfin.twse.com.tw", &server.uri())],
        visit("2330"),
    )
    .await
}

/// 依年度排序；2024 年有綜合損益表算出每股稅前淨利，2023 年抓不到就留 0。
#[tokio::test]
async fn visit_assembles_annual_profits_from_series_and_reports() {
    let server = MockServer::start().await;
    mount_series(&server, "EPS", [32.34, 40.0]).await;
    mount_series(&server, "Revenue", [2_000_000.0, 3_000_000.0]).await;
    mount_series(&server, "CommonStock", [100_000.0, 100_000.0]).await;
    Mock::given(method("POST"))
        .and(path("/compare/report"))
        .and(body_string_contains("ys=20244"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(REPORT_2024, "text/html; charset=utf-8"),
        )
        .mount(&server)
        .await;

    let profits = visit_mocked(&server).await.expect("MOPS 有資料");

    let years: Vec<i32> = profits.iter().map(|profit| profit.year).collect();
    assert_eq!(years, [2023, 2024]);
    assert_eq!(profits[0].earnings_per_share, dec!(32.34));
    assert_eq!(
        profits[0].profit_before_tax,
        Decimal::ZERO,
        "2023 年沒有損益表"
    );
    assert!(profits[0].sales_per_share > Decimal::ZERO);
    assert!(profits[1].profit_before_tax > Decimal::ZERO);
}

/// EPS 沒有任何資料點時直接回錯誤，不再查營收與股本。
#[tokio::test]
async fn visit_fails_when_eps_series_is_empty() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/compare/data"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "xaxisList": [],
            "graphData": [{ "data": [] }]
        })))
        .mount(&server)
        .await;

    let error = visit_mocked(&server).await.expect_err("EPS 是空的");

    assert!(error.to_string().contains("response is empty"), "{error:#}");
    let requests = server.received_requests().await.unwrap_or_default();
    assert_eq!(requests.len(), 1, "只查 EPS");
}
