//! # 上櫃次日漲跌停價
//!
//! 來源：TPEx `www/zh-tw/afterTrading/otc?type=EW&response=json`
//! （上櫃股票每日收盤行情，不含權證、牛熊證，約 147 KB）。不帶 `date` 時回最新一個交易日，
//! 最後兩欄是「次日漲停價」「次日跌停價」，適用於資料日期的下一個交易日。
//!
//! 回應是 `{"tables":[{"date":"115/11/06","fields":[…],"data":[[…]]}]}`；欄位名稱尾端可能有空白。
//! 沒有漲跌幅限制的證券（債券 ETF、國外成分 ETF 等）次日漲停寫 `9,999.95`、次日跌停寫 `0.01`。

use anyhow::{Result, anyhow};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::{
    core::util::{self, datetime::roc_year_to_gregorian_year},
    infra::crawler::{share::ExchangePriceLimits, tpex},
};

/// 每日收盤行情的 JSON 回應。
#[derive(Debug, Default, Deserialize)]
struct Response {
    #[serde(default)]
    tables: Vec<Table>,
}

/// 回應中的一張表。
#[derive(Debug, Default, Deserialize)]
struct Table {
    #[serde(default)]
    date: String,
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default)]
    data: Vec<Vec<serde_json::Value>>,
}

/// 取得最新一個交易日收盤行情附帶的上櫃次日漲跌停價。
///
/// # Errors
///
/// 請求失敗、沒有資料表、日期無法解析，或缺少代號／次日漲跌停欄位時回傳錯誤。
pub async fn visit() -> Result<ExchangePriceLimits> {
    let url = format!(
        "https://{host}/www/zh-tw/afterTrading/otc?type=EW&response=json",
        host = tpex::HOST
    );
    let response = util::http::get_json::<Response>(&url).await?;
    parse(response)
}

/// 解析民國日期 `115/11/06`。
fn parse_roc_date(text: &str) -> Option<NaiveDate> {
    let mut parts = text.trim().split('/');
    let year = roc_year_to_gregorian_year(parts.next()?.parse().ok()?);
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    NaiveDate::from_ymd_opt(year, month, day)
}

/// 解析收盤行情回應。
fn parse(response: Response) -> Result<ExchangePriceLimits> {
    let table = response
        .tables
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("上櫃收盤行情沒有資料表"))?;
    let date = parse_roc_date(&table.date)
        .ok_or_else(|| anyhow!("上櫃收盤行情日期 {:?} 無法解析", table.date))?;
    let column = |name: &str| {
        table
            .fields
            .iter()
            .position(|field| field.trim() == name)
            .ok_or_else(|| anyhow!("上櫃收盤行情缺少「{name}」欄位"))
    };
    let (symbol, up, down) = (
        column("代號")?,
        column("次日漲停價")?,
        column("次日跌停價")?,
    );
    let limits = table
        .data
        .iter()
        .filter_map(|row| ExchangePriceLimits::row(row, symbol, down, up))
        .collect();
    Ok(ExchangePriceLimits { date, limits })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::cache::PriceLimit;

    fn body(date: &str, rows: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "tables": [{
                "title": "上櫃股票每日收盤行情(不含定價)",
                "date": date,
                "fields": ["代號","名稱","收盤 ","漲跌","次日漲停價 ","次日跌停價"],
                "data": rows,
            }]
        })
    }

    /// 欄位名稱尾端的空白要忽略；債券 ETF 是無限制、一般股票是區間。
    #[test]
    fn parses_next_day_limits() {
        let response: Response = serde_json::from_value(body(
            "115/11/06",
            serde_json::json!([
                [
                    "00679B",
                    "元大美債20年",
                    "26.89",
                    "-0.10",
                    "9,999.95",
                    "0.01"
                ],
                ["6488", "環球晶", "512.00", "-10.00", "563.00", "461.00"],
                ["8888", "壞列", "1", "0", "--", "--"]
            ]),
        ))
        .expect("測試回應格式正確");
        let parsed = parse(response).expect("格式正確要解析成功");

        assert_eq!(
            parsed.date,
            NaiveDate::from_ymd_opt(2026, 11, 6).expect("valid date")
        );
        assert_eq!(
            parsed.limits,
            vec![
                ("00679B".to_owned(), PriceLimit::Unlimited),
                (
                    "6488".to_owned(),
                    PriceLimit::Range {
                        down: "461.00".parse().expect("decimal"),
                        up: "563.00".parse().expect("decimal"),
                    }
                ),
            ]
        );
    }

    /// 沒有資料表、日期壞掉、缺欄位都要回錯誤。
    #[test]
    fn rejects_bad_responses() {
        let empty: Response =
            serde_json::from_value(serde_json::json!({ "tables": [] })).expect("測試回應格式正確");
        assert!(parse(empty).is_err());

        let bad_date: Response = serde_json::from_value(body("2026-11-06", serde_json::json!([])))
            .expect("測試回應格式正確");
        assert!(parse(bad_date).is_err());

        let missing: Response = serde_json::from_value(serde_json::json!({
            "tables": [{ "date": "115/11/06", "fields": ["代號", "次日漲停價"], "data": [] }]
        }))
        .expect("測試回應格式正確");
        let message = parse(missing).expect_err("缺次日跌停價").to_string();
        assert!(message.contains("次日跌停價"), "{message}");

        assert_eq!(parse_roc_date("115/11/06/1"), None);
        assert_eq!(parse_roc_date("115/13/06"), None);
    }

    /// visit 打正式網址的路徑與參數（mock，不連外）。
    #[tokio::test]
    async fn visit_requests_otc_close_quotes() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/www/zh-tw/afterTrading/otc"))
            .and(query_param("type", "EW"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body(
                "115/11/06",
                serde_json::json!([[
                    "00937B",
                    "群益ESG投等債20+",
                    "14.47",
                    "0",
                    "9,999.95",
                    "0.01"
                ]]),
            )))
            .expect(1)
            .mount(&server)
            .await;

        let parsed = util::http::mock_origin::with_origins(
            &[("https://www.tpex.org.tw", &server.uri())],
            visit(),
        )
        .await
        .expect("mock 回應要解析成功");
        assert_eq!(
            parsed.limits,
            vec![("00937B".to_owned(), PriceLimit::Unlimited)]
        );
    }
}
