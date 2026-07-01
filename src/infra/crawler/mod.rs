//! # 股票資料採集模組 (Crawler Module)
//!
//! 此模組負責從各種外部來源（如證交所、櫃買中心、財經入口網站等）採集股票相關資料。
//! 它整合了多個不同的採集站點，並提供統一的介面供系統其他部分使用。
//!
//! ## 主要功能
//!
//! - **多站點支援**：整合了 Yahoo 財經、鉅亨網、CMoney、PCHome 等多個來源。
//! - **負載平衡與備援**：使用輪詢 (Round-robin) 機制切換不同站點，並在主站點失敗時自動嘗試備援站點。
//! - **抽象化介面**：透過 `StockInfo` Trait 定義統一的資料獲取行為。
//! - **站點池集中管理**：將站點名稱與抓取函式綁成模組層級清單，降低平行陣列對不齊的維護風險。

use std::sync::OnceLock;

use anyhow::Result;
use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::{core::declare, core::util};

/// 臺灣銀行 (提供匯率、財務報表等資料)
pub mod bank_of_taiwan;
/// IP 資訊服務 (BigDataCloud)
pub mod bigdatacloud;
/// 理財寶 - 股市爆料同學會 (提供即時股價與社群資訊)
pub mod cmoney;
/// 鉅亨網 (提供財經新聞與即時報價)
pub mod cnyes;
/// 富邦證券
pub mod fbs;
/// Fugle 行情 API
pub mod fugle;
/// Goodinfo! 台灣股市資訊網 (提供股利與基本面資料)
pub mod goodinfo;
/// HiStock 嗨投資 (財經社群與數據站)
pub mod histock;
/// IP 檢測服務 (ipconfig.io)
pub mod ipconfig;
/// IP 檢測服務 (ipify)
pub mod ipify;
/// IP 資訊查詢 (ipinfo)
pub mod ipinfo;
/// PCHOME 股市 (提供即時行情)
pub mod megatime;
/// MoneyDJ 理財網 (嘉實資訊，提供詳盡的財務指標)
pub mod moneydj;
/// 公開資訊觀測站（MOPS）/ 財務比較 E 點通
pub mod mops;
/// IP 檢測服務 (MyIP)
pub mod myip;
/// NStock 恩投資 (提供 EPS 與各類統計數據)
pub mod nstock;
/// 即時價格背景任務協調層
pub mod price_tasks;
/// IP 檢測服務 (SeeIP)
pub mod seeip;
/// 內部共享模組，包含多個來源共用的解析邏輯 (如元大、嘉實、富邦)
pub mod share;
/// 站點池與股價聚合（多來源輪詢、備援與延遲統計）
mod site_pool;
/// 臺灣期貨交易所 (TAIFEX)
pub mod taifex;
/// 臺灣證券櫃檯買賣中心 (TPEX, 指數與上櫃股票資料)
pub mod tpex;
/// 臺灣證券交易所 (TWSE, 上市股票核心資料來源)
pub mod twse;
/// 撿股讚 (提供股利與選股資料)
pub mod wespai;
/// Winvest (提供即時行情與多維度個股資料)
pub mod winvest;
/// Yahoo 財經 (國際與台灣股市即時行情)
pub mod yahoo;
/// 元大證券 (提供技術面與基本面資料)
pub mod yuanta;

pub use site_pool::{
    FetchedStockPrice, fetch_stock_price_from_backup_sites,
    fetch_stock_price_from_backup_sites_with_source, fetch_stock_price_from_remote_site,
    fetch_stock_quotes_from_remote_site, flush_site_latency_stats,
};

/// 爬蟲層結構化錯誤類型。
#[derive(Debug, thiserror::Error)]
pub enum CrawlerError {
    /// HTTP 請求失敗。
    #[error("network error: {0}")]
    Network(String),

    /// CSS Selector 建構或 HTML 解析失敗。
    #[error("scraper error: {0}")]
    Scraper(String),

    /// 數值、日期或資料格式解析失敗。
    #[error("parse error: {0}")]
    Parse(String),

    /// JSON 反序列化失敗。
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// 回應資料為空或所有來源均失敗。
    #[error("empty response: {0}")]
    EmptyResponse(String),
}

/// `StockInfo` Trait 定義了股票採集器必須實作的基本行為。
///
/// 任何想要加入採集序列的站點都應實作此介面，以確保能被統一調度。
#[async_trait]
pub trait StockInfo {
    /// 獲取指定股票代碼的最新成交價。
    ///
    /// # 參數
    /// * `stock_symbol` - 股票代碼 (例如: "2330")
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal>;

    /// 獲取指定股票代碼的完整報價資訊（包含開高低收、漲跌幅等）。
    ///
    /// # 參數
    /// * `stock_symbol` - 股票代碼 (例如: "2330")
    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes>;
}

#[cfg(test)]
pub(crate) async fn log_stock_price_test<S>(stock_symbol: &str)
where
    S: StockInfo,
{
    tracing::debug!("開始 get_stock_price");

    match S::get_stock_price(stock_symbol).await {
        Ok(price) => {
            dbg!(&price);
            tracing::debug!("price : {:#?}", price);
        }
        Err(why) => {
            tracing::debug!("Failed to get_stock_price because {:?}", why);
        }
    }

    tracing::debug!("結束 get_stock_price");
}

#[cfg(test)]
pub(crate) async fn log_public_ip_visit_test<F, Fut>(visit: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    match visit().await {
        Ok(ip) => {
            dbg!(ip);
        }
        Err(why) => {
            tracing::error!("Failed to get because {:?}", why);
        }
    }
}

/// 讀取回傳純文字 IP 的 public IP endpoint。
pub(crate) async fn get_public_ip_text(
    url_cache: &OnceLock<String>,
    host: &str,
    path: &str,
    trim: bool,
) -> Result<String, CrawlerError> {
    let url = url_cache.get_or_init(|| format!("https://{host}{path}"));
    let ip = util::http::get(url, None)
        .await
        .map_err(|e| CrawlerError::Network(e.to_string()))?;

    if trim {
        Ok(ip.trim().to_string())
    } else {
        Ok(ip)
    }
}
