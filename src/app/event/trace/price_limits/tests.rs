//! 開盤前載入交易所公告的漲跌停價（H46）的測試。

use rust_decimal::Decimal;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::core::util::http::mock_origin::with_origins;

fn date(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, day).expect("valid date")
}

fn range(down: i64, up: i64) -> PriceLimit {
    PriceLimit::Range {
        down: Decimal::new(down, 2),
        up: Decimal::new(up, 2),
    }
}

/// 資料是今天的才採用區間；不是今天的只留無漲跌幅限制。
#[test]
fn ranges_only_when_data_is_for_today() {
    let data = || ExchangePriceLimits {
        date: date(6),
        limits: vec![
            ("00954".to_owned(), PriceLimit::Unlimited),
            ("1718".to_owned(), range(881, 1075)),
        ],
    };

    let (limits, applied) = usable_limits(data(), true);
    assert_eq!(limits.len(), 2);
    assert_eq!(
        applied,
        Applied {
            unlimited: 1,
            ranges: 1
        }
    );

    let (limits, applied) = usable_limits(data(), false);
    assert_eq!(limits, vec![("00954".to_owned(), PriceLimit::Unlimited)]);
    assert_eq!(
        applied,
        Applied {
            unlimited: 1,
            ranges: 0
        }
    );
}

fn twse_body(data_date: &str, unlimited: &str, ranged: &str) -> serde_json::Value {
    serde_json::json!({
        "stat": "OK", "date": data_date,
        "fields": ["證券代號","證券名稱","漲停價","開盤競價基準","跌停價"],
        "data": [
            [unlimited, "測試無限制", "9,999.95", "14.21", "0.01"],
            [ranged, "測試區間", "10.75", "9.78", "8.81"]
        ]
    })
}

fn tpex_body() -> serde_json::Value {
    serde_json::json!({
        "tables": [{
            "date": "115/11/05",
            "fields": ["代號","名稱","次日漲停價 ","次日跌停價"],
            "data": [
                ["79985", "測試債券 ETF", "9,999.95", "0.01"],
                ["79986", "測試上櫃區間", "563.00", "461.00"]
            ]
        }]
    })
}

async fn mount(server: &MockServer, route: &str, status: u16, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

/// 上市資料是今天的：無限制與區間都寫入；上櫃只寫無限制（次日區間不採用）。
/// 用 1991 年的日期，不會蓋到今天盤中真正在用的限制。
#[tokio::test]
async fn execute_writes_today_limits_from_both_markets() {
    let today = NaiveDate::from_ymd_opt(1991, 1, 3).expect("valid date");
    let twse = MockServer::start().await;
    let tpex = MockServer::start().await;
    mount(
        &twse,
        "/rwd/zh/variation/TWT84U",
        200,
        twse_body("19910103", "79983", "79984"),
    )
    .await;
    mount(&tpex, "/www/zh-tw/afterTrading/otc", 200, tpex_body()).await;

    with_origins(
        &[
            ("https://www.twse.com.tw", &twse.uri()),
            ("https://www.tpex.org.tw", &tpex.uri()),
        ],
        execute_on(today),
    )
    .await
    .expect("兩個市場都成功");

    assert_eq!(SHARE.price_limit_on("79983", today), PriceLimit::Unlimited);
    assert_eq!(SHARE.price_limit_on("79984", today), range(881, 1075));
    assert_eq!(SHARE.price_limit_on("79985", today), PriceLimit::Unlimited);
    assert_eq!(SHARE.price_limit_on("79986", today), PriceLimit::Unknown);
}

/// 上市資料還是前一個交易日的：只寫無限制；上櫃失敗不影響上市。兩邊都失敗才回錯誤。
#[tokio::test]
async fn execute_keeps_unlimited_only_for_stale_listed_data() {
    let today = NaiveDate::from_ymd_opt(1991, 1, 4).expect("valid date");
    let twse = MockServer::start().await;
    let tpex = MockServer::start().await;
    mount(
        &twse,
        "/rwd/zh/variation/TWT84U",
        200,
        twse_body("19910103", "79987", "79988"),
    )
    .await;
    mount(
        &tpex,
        "/www/zh-tw/afterTrading/otc",
        404,
        serde_json::json!({}),
    )
    .await;

    let origins = [
        ("https://www.twse.com.tw", twse.uri()),
        ("https://www.tpex.org.tw", tpex.uri()),
    ];
    let origins = origins
        .each_ref()
        .map(|(origin, uri)| (*origin, uri.as_str()));
    with_origins(&origins, execute_on(today))
        .await
        .expect("上市成功就不算失敗");
    assert_eq!(SHARE.price_limit_on("79987", today), PriceLimit::Unlimited);
    assert_eq!(SHARE.price_limit_on("79988", today), PriceLimit::Unknown);

    let down = MockServer::start().await;
    let message = with_origins(
        &[
            ("https://www.twse.com.tw", &down.uri()),
            ("https://www.tpex.org.tw", &down.uri()),
        ],
        execute_on(today),
    )
    .await
    .expect_err("兩個市場都失敗要回錯誤")
    .to_string();
    assert!(message.contains("交易所漲跌停價載入失敗"), "{message}");
}
