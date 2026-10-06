//! # 上櫃股票的三大法人買賣超與融資融券餘額
//!
//! | 函式 | 來源 |
//! |------|------|
//! | [`visit_institutional`] | 櫃買 `3insti/daily_trade/3itrade_hedge_result.php`（單位：股） |
//! | [`visit_margin`] | 櫃買 `margin_trading/margin_balance/margin_bal_result.php`（單位：張） |
//!
//! 兩者一次回傳全部上櫃股票，日期用民國年（`115/10/05`）；休市或尚未公布時沒有資料。

use std::collections::HashMap;

use anyhow::Result;
use chrono::{Datelike, NaiveDate};
use serde::Deserialize;

use crate::{
    core::util,
    infra::crawler::{
        share::{
            InstitutionalFlow, MarginBalance,
            chip::{collect_rows, parse_count},
        },
        tpex,
    },
};

/// 櫃買的回應：`tables[0].data` 是資料列。
#[derive(Debug, Default, Deserialize)]
struct TpexResponse {
    #[serde(default)]
    tables: Vec<TpexTable>,
}

#[derive(Debug, Default, Deserialize)]
struct TpexTable {
    #[serde(default)]
    data: Option<Vec<Vec<String>>>,
}

impl TpexResponse {
    fn rows(self) -> Vec<Vec<String>> {
        self.tables
            .into_iter()
            .next()
            .and_then(|table| table.data)
            .unwrap_or_default()
    }
}

/// 民國日期參數（`115/10/05`）。
fn roc_date(date: NaiveDate) -> String {
    format!(
        "{}/{}",
        util::datetime::gregorian_year_to_roc_year(date.year()),
        date.format("%m/%d")
    )
}

/// 取得指定交易日上櫃股票的三大法人買賣超（`代號 → 買賣超`）。
///
/// # Errors
///
/// HTTP 請求或 JSON 反序列化失敗時回傳錯誤；休市或尚未公布回傳空集合。
pub async fn visit_institutional(date: NaiveDate) -> Result<HashMap<String, InstitutionalFlow>> {
    let url = format!(
        "https://{}/web/stock/3insti/daily_trade/3itrade_hedge_result.php?l=zh-tw&se=EW&t=D&d={}",
        tpex::HOST,
        roc_date(date)
    );
    let response = util::http::get_json::<TpexResponse>(&url).await?;
    Ok(parse_institutional(response))
}

/// 取得指定交易日上櫃股票的融資融券餘額（`代號 → 餘額`）。
///
/// # Errors
///
/// HTTP 請求或 JSON 反序列化失敗時回傳錯誤；休市或尚未公布回傳空集合。
pub async fn visit_margin(date: NaiveDate) -> Result<HashMap<String, MarginBalance>> {
    let url = format!(
        "https://{}/web/stock/margin_trading/margin_balance/margin_bal_result.php?l=zh-tw&d={}",
        tpex::HOST,
        roc_date(date)
    );
    let response = util::http::get_json::<TpexResponse>(&url).await?;
    Ok(parse_margin(response))
}

/// 欄位（24 欄）：10 外資及陸資合計買賣超、13 投信買賣超、22 自營商合計買賣超。
fn parse_institutional(response: TpexResponse) -> HashMap<String, InstitutionalFlow> {
    collect_rows(&response.rows(), |row| {
        Some(InstitutionalFlow {
            foreign: parse_count(row.get(10)?)?,
            trust: parse_count(row.get(13)?)?,
            dealer: parse_count(row.get(22)?)?,
        })
    })
}

/// 欄位：2 前資餘額、6 資餘額、10 前券餘額、14 券餘額（單位：張）。
fn parse_margin(response: TpexResponse) -> HashMap<String, MarginBalance> {
    collect_rows(&response.rows(), |row| {
        Some(MarginBalance {
            margin_previous: parse_count(row.get(2)?)?,
            margin_today: parse_count(row.get(6)?)?,
            short_previous: parse_count(row.get(10)?)?,
            short_today: parse_count(row.get(14)?)?,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTITUTIONAL_FIXTURE: &str = include_str!("testdata/institutional_3itrade.json");
    const MARGIN_FIXTURE: &str = include_str!("testdata/margin_balance.json");

    #[test]
    fn roc_date_formats_the_query_parameter() {
        assert_eq!(
            roc_date(NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()),
            "115/10/05"
        );
    }

    /// 5347 2026-10-05：外資合計 −2,851,652 股、投信 350,024、自營合計 −44,678，合計等於最後一欄。
    #[test]
    fn parse_institutional_fixture() {
        let flows =
            parse_institutional(serde_json::from_str(INSTITUTIONAL_FIXTURE).expect("fixture"));
        assert_eq!(flows.len(), 3);
        let vis = flows["5347"];
        assert_eq!(
            vis,
            InstitutionalFlow {
                foreign: -2_851_652,
                trust: 350_024,
                dealer: -44_678,
            }
        );
        assert_eq!(vis.total(), -2_546_306);
    }

    #[test]
    fn parse_margin_fixture() {
        let balances = parse_margin(serde_json::from_str(MARGIN_FIXTURE).expect("fixture"));
        assert_eq!(balances.len(), 3);
        assert_eq!(
            balances["6488"],
            MarginBalance {
                margin_previous: 16_966,
                margin_today: 17_449,
                short_previous: 714,
                short_today: 715,
            }
        );
    }

    #[test]
    fn empty_tables_yield_empty_maps() {
        assert!(parse_institutional(serde_json::from_str(r#"{"tables":[]}"#).unwrap()).is_empty());
        assert!(parse_margin(serde_json::from_str(r#"{"stat":"ok"}"#).unwrap()).is_empty());
    }

    #[tokio::test]
    #[ignore = "live test：連線真實外部網站，需要時手動執行"]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        let date = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let flows = visit_institutional(date).await.expect("三大法人");
        let margins = visit_margin(date).await.expect("融資融券");
        dbg!(
            flows.len(),
            margins.len(),
            flows.get("6488"),
            margins.get("6488")
        );
        assert!(flows.len() > 500 && margins.len() > 500);
    }
}
