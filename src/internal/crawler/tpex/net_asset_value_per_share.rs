use crate::internal::{crawler::tpex, util};
use anyhow::*;
use rust_decimal::Decimal;
use scraper::{Html, Selector};
use std::{collections::HashMap, result::Result::Ok, str::FromStr};

#[derive(Default, Debug, Clone, PartialEq)]
//#[serde(rename_all = "camelCase")]
pub struct Emerging {
    pub stock_symbol: String,
    pub net_asset_value_per_share: Decimal,
}

impl Emerging {
    pub fn new(stock_symbol: String, net_asset_value_per_share: Decimal) -> Self {
        Emerging {
            stock_symbol,
            net_asset_value_per_share,
        }
    }
}

pub async fn visit() -> Result<Vec<Emerging>> {
    let url = format!(
        "https://{}/web/regular_emerging/corporateInfo/emerging/emerging_stock.php?l=zh-tw",
        tpex::HOST
    );

    //choice_type=stk_market&stk_market=ALL&stk_code=&stk_category=02&stk_type=
    let mut params = HashMap::new();
    params.insert("choice_type", "stk_market");
    params.insert("stk_market", "ALL");
    params.insert("stk_category", "02");

    let response = util::http::post(&url, None, Some(params)).await?;
    let mut result: Vec<Emerging> = Vec::with_capacity(512);
    let document = Html::parse_document(&response);
    //#company_list > tbody > tr:nth-child(1)
    if let Ok(selector) = Selector::parse("#company_list > tbody > tr") {
        for node in document.select(&selector) {
            let tds: Vec<&str> = node.text().map(str::trim).collect();
            if tds.len() < 5 {
                continue;
            }

            let e = Emerging::new(
                tds[1].to_string(),
                Decimal::from_str(tds[5]).unwrap_or_default(),
            );
            result.push(e);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use crate::{internal::cache::SHARE, internal::logging};
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 visit".to_string());

        match visit().await {
            Err(why) => {
                logging::error_file_async(format!("Failed to visit because {:?}", why));
            }
            Ok(list) => {
                logging::debug_file_async(format!("data({}):{:#?}", list.len(), list));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
