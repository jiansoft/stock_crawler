//! 公開資訊觀測站季 EPS 爬蟲。
//!
//! 此模組負責向公開資訊觀測站請求指定市場、年度與季度的季 EPS 清單，
//! 並解析成 [`Eps`] 結構。

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
/// 單一股票於指定年度與季度的 EPS 資料。
pub struct Eps {
    /// 年度
    pub year: i32,
    /// 季度 Q4 Q3 Q2 Q1
    pub quarter: Quarter,
    /// 股票代號。
    pub stock_symbol: String,
    /// 每股稅後淨利
    pub earnings_per_share: Decimal,
}

impl Eps {
    /// 建立一筆季 EPS 資料。
    pub fn new(stock_symbol: String, year: i32, quarter: Quarter, eps: Decimal) -> Self {
        Self {
            year,
            quarter,
            stock_symbol,
            earnings_per_share: eps,
        }
    }
}

/// 向公開資訊觀測站抓取指定市場與季度的 EPS 清單。
///
/// # 參數
///
/// * `stock_exchange_market` - 市場別，例如上市或上櫃
/// * `year` - 目標財報年度（西元年）
/// * `quarter` - 目標財報季度
///
/// # 回傳值
///
/// 成功時回傳符合條件的 [`Eps`] 清單；失敗時回傳錯誤。
///
/// # 錯誤
///
/// 當 HTTP 請求失敗、回應無法解析，或來源站結構異常時回傳錯誤。
pub async fn visit(
    stock_exchange_market: StockExchangeMarket,
    year: i32,
    quarter: Quarter,
) -> Result<Vec<Eps>> {
    let url = format!(
        "https://mopsov.{host}/mops/web/ajax_t163sb19",
        host = twse::HOST,
    );
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
    let selector_table = Selector::parse("table").expect("Failed to parse table selector");
    let selector_tr = Selector::parse("tr").expect("Failed to parse tr selector");
    let selector_td = Selector::parse("td").expect("Failed to parse td selector");
    for table in document.select(&selector_table) {
        for tr in table.select(&selector_tr) {
            let tds: Vec<_> = tr
                .select(&selector_td)
                .map(|td| td.text().collect::<String>().trim().to_string())
                .collect();

            if tds.len() != 9 {
                continue;
            }

            let stock_symbol = &tds[0];

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
                tds[3].to_string().get_decimal(None),
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

        match visit(StockExchangeMarket::Listed, 2025, Quarter::Q4).await {
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
