use std::{collections::HashMap, str::FromStr};

use anyhow::Result;
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use crate::{core::util, infra::crawler::tpex};

#[derive(Default, Debug, Clone, PartialEq)]
//#[serde(rename_all = "camelCase")]
/// 興櫃公司每股淨值資料。
pub struct Emerging {
    /// 股票代號。
    pub stock_symbol: String,
    /// 每股淨值。
    pub net_asset_value_per_share: Decimal,
}

impl Emerging {
    /// 建立一筆興櫃公司每股淨值資料。
    pub fn new(stock_symbol: String, net_asset_value_per_share: Decimal) -> Self {
        Emerging {
            stock_symbol,
            net_asset_value_per_share,
        }
    }
}

/// 抓取興櫃公司每股淨值清單。
///
/// # 錯誤
///
/// 當 HTTP 請求失敗或頁面解析失敗時回傳錯誤。
pub async fn visit() -> Result<Vec<Emerging>> {
    let url = format!(
        "https://{}/web/regular_emerging/corporateInfo/emerging/emerging_stock.php?l=zh-tw",
        tpex::HOST
    );

    let mut params = HashMap::new();
    params.insert("choice_type", "stk_market");
    params.insert("stk_market", "ALL");
    params.insert("stk_category", "02");

    let response = util::http::post(&url, None, Some(params)).await?;
    Ok(parse_emerging_html(&response))
}

/// 從興櫃公司每股淨值頁面 HTML 解析出 `Emerging` 清單。
///
/// 欄位佈局（以 text-node 計）：序號, 股票代號, 公司名稱, 股數, 盈虧, 每股淨值, …
/// 需要至少 6 個 text node 才能安全取得 `tds[5]`（每股淨值）。
fn parse_emerging_html(html: &str) -> Vec<Emerging> {
    let mut result = Vec::with_capacity(512);
    let document = Html::parse_document(html);
    let Ok(selector) = Selector::parse("#company_list > tbody > tr") else {
        return result;
    };
    for node in document.select(&selector) {
        let tds: Vec<&str> = node.text().map(str::trim).collect();
        if tds.len() < 6 {
            continue;
        }
        result.push(Emerging::new(
            tds[1].to_string(),
            Decimal::from_str(tds[5]).unwrap_or_default(),
        ));
    }
    result
}

#[cfg(test)]
mod tests;
