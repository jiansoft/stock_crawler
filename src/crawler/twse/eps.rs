use std::collections::HashMap;

use anyhow::{anyhow, Result};
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use crate::{
    cache::SHARE,
    crawler::twse,
    declare::{Quarter, StockExchangeMarket},
    util::{self, convert::FromValue, datetime},
};

#[derive(Debug, Clone)]
/// 財務報表
pub struct Eps {
    /// 年度
    pub year: i32,
    /// 季度 Q4 Q3 Q2 Q1
    pub quarter: Quarter,
    pub stock_symbol: String,
    /// 每股稅後淨利
    pub earnings_per_share: Decimal,
}

impl Eps {
    pub fn new(stock_symbol: String, year: i32, quarter: Quarter, eps: Decimal) -> Self {
        Self {
            year,
            quarter,
            stock_symbol,
            earnings_per_share: eps,
        }
    }
}

pub async fn visit(
    stock_exchange_market: StockExchangeMarket,
    year: i32,
    quarter: Quarter,
) -> Result<Vec<Eps>> {
    let url = format!("https://mops.{host}/mops/web/t163sb19", host = twse::HOST,);
    let roc_year = datetime::gregorian_year_to_roc_year(year).to_string();
    let season = format!("0{season}", season = quarter.serial());
    let typek = match stock_exchange_market {
        StockExchangeMarket::Public => "pub",
        StockExchangeMarket::Listed => "sii",
        StockExchangeMarket::OverTheCounter => "otc",
        StockExchangeMarket::Emerging => "rotc",
    };
    let mut params = HashMap::with_capacity(7);
    params.insert("encodeURIComponent", "1");
    params.insert("step", "1");
    params.insert("firstin", "1");
    params.insert("year", &roc_year);
    params.insert("season", &season);
    params.insert("code", "");
    params.insert("TYPEK", typek);

    let response = util::http::post(&url, None, Some(params))
        .await
        .map_err(|err| anyhow!("HTTP request failed: {}", err))?;
    let document = Html::parse_document(&response);
    let mut result = Vec::with_capacity(1024);
    let selector_table =
        Selector::parse("table").map_err(|_| anyhow!("Failed to parse table selector"))?;
    let selector_tr = Selector::parse("tr").map_err(|_| anyhow!("Failed to parse tr selector"))?;

    for table in document.select(&selector_table) {
        for tr in table.select(&selector_tr) {
            let tds: Vec<&str> = tr.text().map(str::trim).collect();
            if tds.len() != 19 {
                continue;
            }

            let stock_symbol = tds[1];

            if stock_symbol.is_empty() {
                continue;
            }

            if !SHARE.stock_contains_key(stock_symbol) {
                continue;
            }
            
            let eps = Eps::new(
                stock_symbol.to_string(),
                year,
                quarter,
                tds[7].to_string().get_decimal(None),
            );

            result.push(eps);
        }
    }
    
    Ok(result)
}

#[cfg(test)]
mod tests {
    use crate::cache::SHARE;
    use crate::logging;

    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 visit".to_string());

        match visit(StockExchangeMarket::Listed, 2023, Quarter::Q4).await {
            Ok(list) => {
                dbg!(&list);
                logging::debug_file_async(format!("list:{:#?}", list));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because: {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
