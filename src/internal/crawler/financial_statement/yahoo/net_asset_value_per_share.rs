use crate::internal::database::model;
use crate::internal::util;
use crate::logging;
use anyhow::*;
use core::result::Result::Ok;
use rust_decimal::Decimal;
use scraper::{Html, Selector};
use std::str::FromStr;

/// 將未下市每股淨值為零的股票試著到yahoo 抓取數據後更新回 stocks表
pub async fn visit(stock_symbol: &str) -> Result<()> {
    let url = format!("https://tw.stock.yahoo.com/quote/{}/profile", stock_symbol);
    let text = util::http::request_get(&url).await?;
    //logging::info_file_async(format!("text {:?}", text));
    //#main-2-QuoteProfile-Proxy > div > section:nth-child(3) > div.table-grid.Mb\\(20px\\).row-fit-half > div:nth-child(6) > div
    //#main-2-QuoteProfile-Proxy > div > section:nth-child(3)
    let document = Html::parse_document(text.as_str());
    let selector = match Selector::parse("#main-2-QuoteProfile-Proxy > div > section:nth-child(3)")
    {
        Ok(selector) => selector,
        Err(e) => return Err(anyhow!("Failed to parse selector: {:?}", e)),
    };

    for (_tr_count, node) in document.select(&selector).enumerate() {
        //println!("node.html():{:?} \r\n",   node.html());
        let tds: Vec<&str> = node.text().collect();
        println!("tds({}):{:#?} \r\n", tds.len(), tds);
        if tds.len() != 2 {
            return Err(anyhow!(
                "Failed to net_asset_value_per_share::visit at parse_document tds is not eq 2:{:?}",
                tds
            ));
        }

        if tds[0] != "每股淨值" {
            return Err(anyhow!(
                "Failed to net_asset_value_per_share::visit tds:{:?}",
                tds
            ));
        }
        let money = tds[1].replace(['元', ' '], "");
        let mut stock = model::stock::Entity::new();
        stock.stock_symbol = stock_symbol.to_string();
        stock.net_asset_value_per_share = Decimal::from_str(&money)?;
        println!("每股淨值:{:?}", stock);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 visit".to_string());

        match visit("2330").await {
            Ok(_) => {}
            Err(why) => {
                logging::error_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::info_file_async("結束 visit".to_string());
    }
}
