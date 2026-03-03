//! # HiStock 即時報價採集
//!
//! 此模組負責透過 HiStock 的站內端點取得台股即時報價資料。
//!
//! 目前使用兩種端點：
//! - `getinfo.asmx/StockLast`：直接回傳最新成交價（純文字）
//! - `stock/module/function.aspx`：回傳即時資訊區塊（HTML 片段），可解析漲跌與漲跌幅

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use crate::{
    crawler::{
        histock::{HiStock, HOST},
        StockInfo,
    },
    declare,
    util::{self, text},
};

/// HiStock 即時報價快照。
///
/// 此結構是將 `function.aspx` 回傳的即時資訊區塊
/// 轉換成程式內部較容易使用的格式。
struct RealtimeSnapshot {
    /// 最新成交價。
    price: f64,
    /// 漲跌金額。
    change: f64,
    /// 漲跌幅（百分比）。
    change_range: f64,
}

/// 透過 HiStock `getinfo.asmx/StockLast` 取得最新成交價。
///
/// # 參數
/// * `stock_symbol` - 股票代號，例如 `6414`
///
/// # 回傳
/// * `Result<Decimal>` - 成功時回傳最新成交價；失敗時回傳解析或連線錯誤
async fn fetch_last_price(stock_symbol: &str) -> Result<Decimal> {
    let url = format!(
        "https://{host}/getinfo.asmx/StockLast?no={symbol}",
        host = HOST,
        symbol = stock_symbol
    );
    let body = util::http::get(&url, None).await?;

    text::parse_decimal(body.trim(), None)
}

/// 透過 HiStock `function.aspx` 取得即時報價摘要。
///
/// 這個端點會回傳一段 HTML 片段與更新時間，
/// 格式為 `HTML~時間字串`，此函式只解析前半段 HTML。
///
/// # 參數
/// * `stock_symbol` - 股票代號，例如 `6414`
///
/// # 回傳
/// * `Result<RealtimeSnapshot>` - 成功時回傳即時報價快照；失敗時回傳解析或連線錯誤
async fn fetch_realtime_snapshot(stock_symbol: &str) -> Result<RealtimeSnapshot> {
    let url = format!("https://{host}/stock/module/function.aspx", host = HOST);
    let mut params = std::collections::HashMap::with_capacity(2);
    params.insert("m", "stocktop2017");
    params.insert("no", stock_symbol);

    let body = util::http::post(&url, None, Some(params)).await?;
    let html = body
        .split('~')
        .next()
        .map(str::trim)
        .unwrap_or_default();
    let document = Html::parse_fragment(html);
    let title_selector = Selector::parse(".ci_title")
        .map_err(|why| anyhow!("Failed to parse HiStock title selector because {:?}", why))?;
    let value_selector = Selector::parse(".ci_value")
        .map_err(|why| anyhow!("Failed to parse HiStock value selector because {:?}", why))?;

    let titles = document
        .select(&title_selector)
        .map(|node| node.text().collect::<String>().trim().to_string())
        .collect::<Vec<_>>();
    let values = document
        .select(&value_selector)
        .map(|node| node.text().collect::<String>().trim().to_string())
        .collect::<Vec<_>>();

    let mut price = None;
    let mut change = None;
    let mut change_range = None;

    for (title, value) in titles.iter().zip(values.iter()) {
        match title.as_str() {
            "股價" => {
                price = Some(text::parse_f64(value, None)?);
            }
            "漲跌" => {
                let is_negative = value.contains('▼');
                let mut parsed = text::parse_f64(value, Some(['▼', '▲'].to_vec()))?;
                if is_negative {
                    parsed = -parsed;
                }
                change = Some(parsed);
            }
            "幅度" => {
                change_range = Some(text::parse_f64(value, None)?);
            }
            _ => {}
        }
    }

    Ok(RealtimeSnapshot {
        price: price.ok_or_else(|| anyhow!("Failed to parse HiStock realtime price"))?,
        change: change.ok_or_else(|| anyhow!("Failed to parse HiStock realtime change"))?,
        change_range: change_range
            .ok_or_else(|| anyhow!("Failed to parse HiStock realtime change range"))?,
    })
}

#[async_trait]
impl StockInfo for HiStock {
    /// 取得指定股票的最新成交價。
    ///
    /// 目前優先使用 `getinfo.asmx/StockLast`，
    /// 避免從整頁 HTML 抓取價格欄位。
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        fetch_last_price(stock_symbol).await
    }

    /// 取得指定股票的即時報價資訊。
    ///
    /// 目前透過 `function.aspx` 解析以下欄位：
    /// - 股價
    /// - 漲跌
    /// - 幅度
    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let snapshot = fetch_realtime_snapshot(stock_symbol).await?;

        Ok(declare::StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price: snapshot.price,
            change: snapshot.change,
            change_range: snapshot.change_range,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;

    /// 驗證 HiStock 最新成交價端點是否可正常取值。
    #[tokio::test]
    async fn test_get_stock_price() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 get_stock_price".to_string());

        match HiStock::get_stock_price("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("price : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to get_stock_price because {:?}", why));
            }
        }

        logging::debug_file_async("結束 get_stock_price".to_string());
    }

    /// 驗證 HiStock 即時報價摘要端點是否可正常解析。
    #[tokio::test]
    async fn test_get_stock_quotes() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 histock::get_stock_quotes".to_string());

        match HiStock::get_stock_quotes("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("histock::get_stock_quotes : {:#?}", e));
            }
            Err(why) => {
                dbg!(&why);
                logging::debug_file_async(format!(
                    "Failed to histock::get_stock_quotes because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 histock::get_stock_quotes".to_string());
    }

    /// 直接驗證 HiStock 兩個即時端點的 live 測試。
    ///
    /// 此測試會真的連到 HiStock，因此以 `#[ignore]` 標記，
    /// 需要時再手動執行。
    #[tokio::test]
    #[ignore]
    async fn test_histock_live_endpoints() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 test_histock_live_endpoints".to_string());

        let price = fetch_last_price("2330").await.unwrap();
        let snapshot = fetch_realtime_snapshot("2330").await.unwrap();

        logging::debug_file_async(format!("histock stock_last price: {}", price));
        logging::debug_file_async(format!("histock realtime snapshot: {:#?}", (
            snapshot.price,
            snapshot.change,
            snapshot.change_range
        )));

        assert!(price > Decimal::ZERO);
        assert!(snapshot.price > 0.0);

        logging::debug_file_async("結束 test_histock_live_endpoints".to_string());
    }
}
