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
mod tests;
