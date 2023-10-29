use core::result::Result::Ok;

use anyhow::*;
use rust_decimal::Decimal;
use scraper::{ElementRef, Html, Selector};

use crate::{
    internal::{crawler::taifex, StockExchange},
    util::{
        self,
        http::element
    }
};

#[derive(Default, Debug, Clone, PartialEq)]
pub struct StockWeight {
    pub rank: i32,
    pub stock_symbol: String,
    pub weight: Decimal,
}

/// 台股各股權重
pub async fn visit(exchange: StockExchange) -> Result<Vec<StockWeight>> {
    let url = match exchange {
        StockExchange::TWSE => {
            format!("https://{}/cht/9/futuresQADetail", taifex::HOST)
        }
        StockExchange::TPEx => {
            format!("https://{}/cht/2/tPEXPropertion", taifex::HOST)
        }
    };

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
        if let Some(sw) = get_stock_weight(
            &element,
            "td:nth-child(1)",
            "td:nth-child(2)",
            "td:nth-child(4)",
        ) {
            result.push(sw);
        }
        if let Some(sw) = get_stock_weight(
            &element,
            "td:nth-child(5)",
            "td:nth-child(6)",
            "td:nth-child(8)",
        ) {
            result.push(sw);
        }
    }

    Ok(result)
}

fn get_stock_weight(element: &ElementRef, ra: &str, sy: &str, w: &str) -> Option<StockWeight> {
    let stock_symbol = element::parse_to_string(element, sy);
    let weight = element::parse_to_decimal(element, w);

    if !stock_symbol.is_empty() && !weight.is_zero() {
        let sw = StockWeight {
            rank: element::parse_to_i32(element, ra),
            stock_symbol,
            weight,
        };

        return Some(sw);
    }

    None
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match visit(StockExchange::TPEx).await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("len:{}\r\n {:#?}", e.len(), e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
