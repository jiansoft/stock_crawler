use anyhow::Result;
use chrono::Local;
use scraper::{Html, Selector};

use crate::{
    core::declare::StockExchangeMarket,
    core::util::{self, datetime::Weekend},
    infra::crawler::twse,
};

const REQUIRED_CATEGORIES: [&str; 4] = ["股票", "特別股", "普通股", "臺灣存託憑證(TDR)"];

/// twse 國際證券識別碼
#[derive(Debug, Clone)]
pub struct InternationalSecuritiesIdentificationNumber {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 國際證券識別碼（ISIN）。
    pub isin_code: String,
    /// 上市日期。
    pub listing_date: String,
    /// 產業分類名稱。
    pub industry: String,
    /// CFI Code。
    pub cfi_code: String,
    /// 交易市場。
    pub market: StockExchangeMarket,
}

/// 調用  twse API 取得台股國際證券識別碼
/// 上市:2 上櫃︰4 興櫃︰5
pub async fn visit(
    mode: StockExchangeMarket,
) -> Result<Vec<InternationalSecuritiesIdentificationNumber>> {
    if Local::now().is_weekend() {
        return Ok(Vec::new());
    }

    let url = format!(
        "https://isin.{}/isin/C_public.jsp?strMode={}",
        twse::HOST,
        mode.serial()
    );

    let response = util::http::get_use_big5(&url).await?;
    let mut result: Vec<InternationalSecuritiesIdentificationNumber> = Vec::with_capacity(4096);
    let document = Html::parse_document(&response);

    if let Ok(selector) = Selector::parse("body > table.h4 > tbody > tr") {
        let mut is_required_category = false;
        for node in document.select(&selector).skip(1) {
            let tds: Vec<&str> = node.text().map(str::trim).collect();
            if tds.len() == 2 {
                is_required_category = REQUIRED_CATEGORIES.contains(&tds[0].trim());
                continue;
            }

            if !is_required_category {
                continue;
            }

            let split: Vec<&str> = tds[0].split('\u{3000}').collect();
            if split.len() != 2 {
                // 名稱和代碼有缺
                continue;
            }

            let industry: String;
            let cfi_code: String;

            match tds.len() {
                5 => {
                    industry = "未分類".to_string();
                    cfi_code = tds[4].to_owned()
                }
                6 => {
                    industry = tds[4].to_owned();
                    cfi_code = tds[5].to_owned();
                }
                _ => {
                    industry = "未分類".to_string();
                    cfi_code = "".to_string();
                }
            };

            let isin = InternationalSecuritiesIdentificationNumber {
                stock_symbol: split[0].trim().to_owned(),
                name: split[1].to_owned(),
                isin_code: tds[1].to_owned(),
                listing_date: tds[2].to_owned(),
                industry,
                cfi_code,
                market: mode,
            };

            result.push(isin);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use crate::core::logging;
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());
        for mode in StockExchangeMarket::iterator() {
            match visit(mode).await {
                Err(why) => {
                    logging::error_file_async(format!("Failed to visit because {:?}", why));
                }
                Ok(result) => {
                    for item in result.iter() {
                        logging::debug_file_async(format!("item:{:#?}", item));
                    }
                }
            }
        }
        logging::debug_file_async("結束 visit".to_string());
    }
}
