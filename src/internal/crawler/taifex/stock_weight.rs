use core::result::Result::Ok;

use anyhow::*;
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use crate::internal::{crawler::taifex, logging, util, util::http::element};

#[derive(Default, Debug, Clone, PartialEq)]
pub struct StockWeight {
    pub rank: i32,
    pub stock_symbol: String,
    pub weight: Decimal,
}

/// 台股各股權重
pub async fn visit() -> Result<Vec<StockWeight>> {
    let url = format!("https://{}/cht/9/futuresQADetail", taifex::HOST);
    logging::info_file_async(format!("visit url:{}", url,));

    let mut result: Vec<StockWeight> = Vec::with_capacity(1024);
    let text = util::http::get(&url, None).await?;
    if text.is_empty() {
        return Ok(result);
    }

    let document = Html::parse_document(text.as_str());
    let selector = match Selector::parse("#printhere > div > table > tbody > tr:not(:first-child)")
    {
        Ok(selector) => selector,
        Err(why) => {
            return Err(anyhow!("Failed to Selector::parse because: {:?}", why));
        }
    };

    for element in document.select(&selector) {
        let odd_e = StockWeight {
            rank: element::parse_to_i32(&element, "td:nth-child(1)"),
            stock_symbol: element::parse_to_string(&element, "td:nth-child(2)"),
            weight: element::parse_to_decimal(&element, "td:nth-child(4)"),
        };

        if !odd_e.stock_symbol.is_empty() {
            result.push(odd_e);
        }

        let even_e = StockWeight {
            rank: element::parse_to_i32(&element, "td:nth-child(5)"),
            stock_symbol: element::parse_to_string(&element, "td:nth-child(6)"),
            weight: element::parse_to_decimal(&element, "td:nth-child(8)"),
        };

        if !even_e.stock_symbol.is_empty() {
            result.push(even_e);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use crate::internal::logging;

    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match visit().await {
            Ok(e) => {
                logging::debug_file_async(format!("len:{}\r\n {:#?}", e.len(), e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
