use core::result::Result::Ok;

use anyhow::{anyhow, Result};
use reqwest::header::HeaderMap;
use rust_decimal::Decimal;
use scraper::{ElementRef, Html, Selector};

use crate::{
    crawler::{taifex, taifex::HOST},
    declare::StockExchange,
    util::{self, http::element},
};

#[derive(Default, Debug, Clone, PartialEq)]
pub struct StockWeight {
    pub rank: i32,
    pub stock_symbol: String,
    pub weight: Decimal,
}

struct ExchangeConfig {
    url: String,
    selector: String,
}

impl ExchangeConfig {
    /// 建立一個新的交易所組態實例。
    ///
    /// # 參數
    ///
    /// * `exchange`: 一個 `StockExchange` 列舉，代表選擇的股票交易所。
    ///
    /// 返回: 返回一個新的 `ExchangeConfig` 實例，其中包含了訪問特定交易所資料所需的URL和選擇器。
    ///
    /// # 示例
    ///
    /// ```rust
    /// let config = ExchangeConfig::new(StockExchange::TWSE);
    /// println!("URL: {}", config.url);
    /// println!("Selector: {}", config.selector);
    /// ```
    ///
    /// 上面的程式碼展示了如何建立一個針對台灣證券交易所 (TWSE) 的組態實例，並列印相關的 URL 和選擇器。
    fn new(exchange: StockExchange) -> Self {
        match exchange {
            StockExchange::TWSE => Self {
                url: format!("https://{}/cht/9/futuresQADetail", taifex::HOST),
                selector: "#printhere > div > div > table > tbody > tr".to_string(),
            },
            StockExchange::TPEx => Self {
                url: format!("https://{}/cht/2/tPEXPropertion", taifex::HOST),
                selector: "#printhere > div > table > tbody > tr".to_string(),
            },
            _ => panic!("Unsupported exchange"),
        }
    }
}

/// 台股各股權重
pub async fn visit(exchange: StockExchange) -> Result<Vec<StockWeight>> {
    let mut result: Vec<StockWeight> = Vec::with_capacity(1024);
    let exchange_market = ExchangeConfig::new(exchange);
    let url = &exchange_market.url;
    let ua = util::http::user_agent::gen_random_ua();
    let mut headers = HeaderMap::new();

    headers.insert("Host", HOST.parse()?);
    headers.insert("Referer", url.parse()?);
    headers.insert("User-Agent", ua.parse()?);

    let text = util::http::get(url, Some(headers)).await?;

    if text.is_empty() {
        return Ok(result);
    }

    let document = Html::parse_document(text.as_str());
    let selector = match Selector::parse(&exchange_market.selector) {
        Ok(selector) => selector,
        Err(why) => {
            return Err(anyhow!("Failed to Selector::parse because: {:?}", why));
        }
    };

    document.select(&selector).for_each(|element| {
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
    });

    Ok(result)
}

/// Parses stock weight from an HTML element.
///
/// # Arguments
/// * `element` - A reference to the element to parse from.
/// * `rank_selector` - CSS selector to find the rank.
/// * `symbol_selector` - CSS selector to find the stock symbol.
/// * `weight_selector` - CSS selector to find the weight.
///
/// # Returns `Some(StockWeight)` if parsing succeeds, otherwise `None`.
fn get_stock_weight(
    element: &ElementRef,
    rank_selector: &str,
    symbol_selector: &str,
    weight_selector: &str,
) -> Option<StockWeight> {
    let stock_symbol = element::parse_to_string(element, symbol_selector);
    let weight = element::parse_to_decimal(element, weight_selector);

    if !stock_symbol.is_empty() && !weight.is_zero() {
        let sw = StockWeight {
            rank: element::parse_to_i32(element, rank_selector),
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

        match visit(StockExchange::TWSE).await {
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
