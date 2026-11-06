//! # 上市股價升降幅度（每日漲跌停價）
//!
//! 來源：TWSE `rwd/zh/variation/TWT84U?response=json&selectType=ALLBUT0999`
//! （股價升降幅度，不含權證，約 136 KB）。不帶 `date` 時回最新一個有資料的交易日，
//! 回應的 `date` 就是這份漲跌停價適用的交易日；當天資料還沒出來時拿到的是前一個交易日，
//! 由呼叫端依資料日期決定能採用哪些欄位。
//!
//! 回應是 `{"stat":"OK","date":"20261106","fields":[…],"data":[[…]]}`；欄位有
//! 「證券代號」「漲停價」「跌停價」（「開盤競價基準」出現兩次，不用）。
//! 沒有漲跌幅限制的證券（國外成分 ETF 等）漲停寫 `9,999.95`、跌停寫 `0.01`。

use anyhow::{Result, anyhow};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::infra::crawler::{share::ExchangePriceLimits, twse};

/// TWT84U 的 JSON 回應。
#[derive(Debug, Default, Deserialize)]
struct Response {
    #[serde(default)]
    stat: String,
    #[serde(default)]
    date: String,
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default)]
    data: Vec<Vec<serde_json::Value>>,
}

/// 取得最新一個交易日的上市漲跌停價。
///
/// # Errors
///
/// 請求失敗、`stat` 不是 `OK`、日期無法解析，或缺少代號／漲跌停欄位時回傳錯誤。
pub async fn visit() -> Result<ExchangePriceLimits> {
    let url = format!(
        "https://www.{host}/rwd/zh/variation/TWT84U?response=json&selectType=ALLBUT0999",
        host = twse::HOST
    );
    let response = crate::core::util::http::get_json::<Response>(&url).await?;
    parse(response)
}

/// 解析 TWT84U 回應。
fn parse(response: Response) -> Result<ExchangePriceLimits> {
    if !response.stat.eq_ignore_ascii_case("OK") {
        return Err(anyhow!("TWT84U 回應 stat={}", response.stat));
    }
    let date = NaiveDate::parse_from_str(&response.date, "%Y%m%d")
        .map_err(|why| anyhow!("TWT84U 日期 {:?} 無法解析: {why}", response.date))?;
    let column = |name: &str| {
        response
            .fields
            .iter()
            .position(|field| field.trim() == name)
            .ok_or_else(|| anyhow!("TWT84U 缺少「{name}」欄位"))
    };
    let (symbol, up, down) = (column("證券代號")?, column("漲停價")?, column("跌停價")?);
    let limits = response
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

    fn response(stat: &str, date: &str, rows: serde_json::Value) -> Response {
        serde_json::from_value(serde_json::json!({
            "stat": stat,
            "date": date,
            "fields": ["證券代號","證券名稱","漲停價","開盤競價基準","跌停價","開盤競價基準","收盤價"],
            "data": rows,
        }))
        .expect("測試回應格式正確")
    }

    /// 一般股票是區間、9,999.95／0.01 是無限制；千分位逗號要去掉，壞列略過。
    #[test]
    fn parses_ranges_unlimited_and_skips_bad_rows() {
        let parsed = parse(response(
            "OK",
            "20261106",
            serde_json::json!([
                ["1718", "中纖", "10.75", "9.78", "8.81", "9.78", "9.29"],
                [
                    "00954",
                    "中信日經高股息",
                    "9,999.95",
                    "14.21",
                    "0.01",
                    "14.21",
                    "13.10"
                ],
                [
                    "2330",
                    "台積電",
                    "1,155.00",
                    "1,050.00",
                    "945.00",
                    "1,050.00",
                    "1,000.00"
                ],
                ["9999", "壞列", "--", "1", "--", "1", "1"],
                ["", "沒代號", "1", "1", "1", "1", "1"]
            ]),
        ))
        .expect("格式正確要解析成功");

        assert_eq!(
            parsed.date,
            NaiveDate::from_ymd_opt(2026, 11, 6).expect("valid date")
        );
        assert_eq!(
            parsed.limits,
            vec![
                (
                    "1718".to_owned(),
                    PriceLimit::Range {
                        down: "8.81".parse().expect("decimal"),
                        up: "10.75".parse().expect("decimal"),
                    }
                ),
                ("00954".to_owned(), PriceLimit::Unlimited),
                (
                    "2330".to_owned(),
                    PriceLimit::Range {
                        down: "945.00".parse().expect("decimal"),
                        up: "1155.00".parse().expect("decimal"),
                    }
                ),
            ]
        );
    }

    /// 查無資料（stat 不是 OK）、日期壞掉、缺欄位都要回錯誤，不能當成「沒有限制」。
    #[test]
    fn rejects_bad_responses() {
        let empty = serde_json::json!([]);
        let message = parse(response("很抱歉，沒有符合條件的資料!", "", empty.clone()))
            .expect_err("stat 不是 OK 要回錯誤")
            .to_string();
        assert!(message.contains("stat="), "{message}");
        assert!(parse(response("OK", "2026-11-06", empty.clone())).is_err());

        let missing: Response = serde_json::from_value(serde_json::json!({
            "stat": "OK", "date": "20261106", "fields": ["證券代號", "漲停價"], "data": []
        }))
        .expect("測試回應格式正確");
        let message = parse(missing).expect_err("缺跌停價欄位").to_string();
        assert!(message.contains("跌停價"), "{message}");
    }

    /// visit 打正式網址的路徑與參數（mock，不連外）。
    #[tokio::test]
    async fn visit_requests_twt84u_without_warrants() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rwd/zh/variation/TWT84U"))
            .and(query_param("selectType", "ALLBUT0999"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "stat": "OK", "date": "20261106",
                "fields": ["證券代號","證券名稱","漲停價","開盤競價基準","跌停價"],
                "data": [["00735", "國泰臺韓科技", "9,999.95", "87.15", "0.01"]]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let parsed = crate::core::util::http::mock_origin::with_origins(
            &[("https://www.twse.com.tw", &server.uri())],
            visit(),
        )
        .await
        .expect("mock 回應要解析成功");
        assert_eq!(
            parsed.limits,
            vec![("00735".to_owned(), PriceLimit::Unlimited)]
        );
    }
}
