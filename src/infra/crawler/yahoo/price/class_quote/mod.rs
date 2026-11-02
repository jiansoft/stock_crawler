//! # Yahoo 類股行情 JSON API
//!
//! 此模組負責與 Yahoo 類股頁背後使用的 JSON API 溝通。
//! 主要工作包含：
//! - 列舉所有需要輪詢的類股分類。
//! - 組裝 `StockServices.getClassQuotes` 的請求 URL。
//! - 處理 `offset` 分頁，直到抓完整個類股。
//! - 將 Yahoo 的原始 JSON 欄位轉成 [`RealtimeSnapshot`]。

use std::{collections::HashMap, time::Duration};

use anyhow::{Context, Result, anyhow};
use once_cell::sync::OnceCell;
use reqwest::{Client, header};
use tokio::time::sleep;

use crate::{
    core::util,
    infra::cache::RealtimeSnapshot,
    infra::crawler::yahoo::{self, YahooClassCategory, YahooClassExchange},
};

/// 回應 DTO 與欄位解析。
mod pages;
mod parse;

#[cfg(test)]
use pages::{ClassQuotesPageResult, parse_class_quotes_page};
use pages::{collect_category_pages, fetch_class_quotes_page};

/// Yahoo 類股行情 JSON API 的基底 URL。
const CLASS_QUOTES_API_URL: &str =
    "https://tw.stock.yahoo.com/_td-stock/api/resource/StockServices.getClassQuotes";
/// Yahoo 類股頁的基底 URL，用來組出請求的 `Referer`。
const CLASS_QUOTES_PAGE_URL: &str = "https://tw.stock.yahoo.com/class-quote";
/// 類股輪詢固定使用的桌面瀏覽器 User-Agent。
///
/// 這裡刻意不用 `util::http::user_agent::gen_random_ua()`：那支會隨機挑到
/// Safari Mobile 或 Samsung Internet 等行動裝置 UA，而本模組請求的是
/// 桌面版類股頁背後的 XHR，UA 與 `Referer` 不一致反而更像機器流量。
/// client 是 `OnceCell`，UA 本來就整個程序固定，隨機也換不到任何好處。
const CLASS_QUOTES_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
/// 所有需要輪詢的 Yahoo 市場類型。
const ALL_CLASS_EXCHANGES: [YahooClassExchange; 3] = [
    YahooClassExchange::Listed,
    YahooClassExchange::OverTheCounter,
    YahooClassExchange::Emerging,
];
/// 同一個類股在抓下一頁前的節流等待時間。
///
/// 類股之間的節流由 `cache.rs` 控制；
/// 這裡只負責單一類股分頁很多時的頁間節流。
const PAGE_REQUEST_INTERVAL: Duration = Duration::from_secs(1);
/// Yahoo 類股 JSON API 專用 client。
///
/// 與全域共用 client 分開，是因為這條輪詢會在同一 host 上連續跑數百秒，
/// 若沿用預設較積極的 keepalive / pool / HTTP2 策略，較容易把 transport 層
/// 的工作集長時間留在 process 內。
static YAHOO_CLASS_QUOTES_CLIENT: OnceCell<Client> = OnceCell::new();

/// 單一類股完整抓取後的摘要。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CategoryFetchDiagnostics {
    /// 實際抓取的頁數。
    pub page_count: usize,
    /// Yahoo 原始回應內累積的 item 數量。
    pub raw_item_count: usize,
    /// 成功解析後的股票數量。
    pub snapshot_count: usize,
}

/// 單一類股完整抓取結果。
pub(crate) struct CategoryFetchResult {
    pub snapshots: HashMap<String, RealtimeSnapshot>,
    pub diagnostics: CategoryFetchDiagnostics,
}

/// 取得 Yahoo 類股 API 專用 client。
///
/// 設計目標是保守控制連線生命週期：
/// - 固定使用 HTTP/1，避免 HTTP/2 連線狀態長期膨脹。
/// - 只保留極少量 idle 連線，讓跨類股輪詢時不會累積過多 transport 狀態。
fn get_yahoo_class_quotes_client() -> Result<&'static Client> {
    YAHOO_CLASS_QUOTES_CLIENT.get_or_try_init(|| {
        util::ensure_rustls_crypto_provider();

        Client::builder()
            .brotli(true)
            .gzip(true)
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(15))
            .tcp_nodelay(true)
            .tcp_keepalive(Duration::from_secs(30))
            .http1_only()
            .pool_max_idle_per_host(1)
            .pool_idle_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::limited(5))
            .referer(true)
            .user_agent(CLASS_QUOTES_USER_AGENT)
            .build()
            .map_err(|e| anyhow!("Failed to create Yahoo class quote client: {:?}", e))
    })
}

/// 列出所有需要由背景任務輪詢的 Yahoo 類股分類。
///
/// 此函式會先攤平三大市場的完整類股字典，再只保留
/// [`YahooClassCategory::collect_enabled`] 為 `true` 的項目。
pub fn all_class_categories() -> Vec<&'static YahooClassCategory> {
    // 先列出三大市場，再把各市場對應的類股清單攤平成單一 Vec，
    // 讓背景任務可以用固定順序逐類股輪詢。
    // 類股字典本身仍保留完整資料，但這裡只會拿 `collect_enabled = true` 的類股，
    // 避免把認購 / 認售 / 指數類這些不需要的分類帶進輪詢工作。
    ALL_CLASS_EXCHANGES
        .into_iter()
        .flat_map(yahoo::class_categories)
        .filter(|category| category.collect_enabled)
        .collect()
}

/// 建立類股的內部識別鍵值。
///
/// 這個鍵值用於快取任務內部追蹤「某個類股上一輪有哪些股票」，
/// 好在下一輪更新時移除該類股已經不存在的股票。
pub fn category_key(category: &YahooClassCategory) -> String {
    // 這裡故意用 `exchange:sector_id` 而不是中文名稱，
    // 因為名稱可能調整，但市場代碼與 sector id 比較穩定，適合當內部鍵值。
    format!("{}:{}", category.exchange.code(), category.sector_id)
}

/// 組出 Yahoo 類股 JSON API 的請求 URL。
///
/// # 參數
/// - `category`: 目標類股分類。
/// - `offset`: 分頁位移，首頁請傳入 `0`。
///
/// # 回傳
/// - 完整的 `StockServices.getClassQuotes` API URL。
pub fn build_class_quotes_api_url(category: &YahooClassCategory, offset: usize) -> String {
    // Yahoo 這支 API 不是一般 query string，而是分號參數格式；
    // 先前實測 query string 會拿到空結果，所以這裡固定產生分號版本。
    format!(
        "{base};exchange={exchange};sectorId={sector_id};offset={offset}",
        base = CLASS_QUOTES_API_URL,
        exchange = category.exchange.code(),
        sector_id = category.sector_id,
    )
}

/// 組出類股頁本身的網址，作為 API 請求的 `Referer`。
///
/// 這支 API 是該頁面的 XHR 端點，正常瀏覽時一定帶著頁面網址當 `Referer`。
pub fn build_class_quotes_page_url(category: &YahooClassCategory) -> String {
    format!(
        "{base}?sectorId={sector_id}&exchange={exchange}",
        base = CLASS_QUOTES_PAGE_URL,
        sector_id = category.sector_id,
        exchange = category.exchange.code(),
    )
}

/// 組出類股 API 請求要帶的標頭。
///
/// 先前這支請求只帶 User-Agent 就送出去，既沒有 `Referer` 也沒有 `Accept`。
/// 注意 `ClientBuilder::referer(true)` 只會在**跟隨轉址時**自動補 `Referer`，
/// 不會替第一次的請求加上——所以必須在這裡自己帶。
fn build_class_quotes_headers(category: &YahooClassCategory) -> Result<header::HeaderMap> {
    let mut headers = header::HeaderMap::new();

    let referer = build_class_quotes_page_url(category);
    headers.insert(
        header::REFERER,
        header::HeaderValue::from_str(&referer)
            .with_context(|| format!("Invalid Yahoo class quote referer: {referer}"))?,
    );
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/json, text/plain, */*"),
    );
    headers.insert(
        header::ACCEPT_LANGUAGE,
        header::HeaderValue::from_static("zh-TW,zh;q=0.9,en;q=0.8"),
    );

    Ok(headers)
}

/// 抓取單一類股的完整即時快照。
///
/// 此函式會從 `offset = 0` 開始，根據 Yahoo API 回傳的 `nextOffset`
/// 持續往後抓，直到整個類股的股票都被收集完成。
/// 若該類股存在多頁資料，分頁與分頁之間會固定等待 1 秒，
/// 避免對同一個 sector 連續請求過密。
///
/// # Errors
/// - 首頁即為空列表。
/// - 分頁 `nextOffset` 格式錯誤。
/// - API 回傳的資料無法解析成有效快照。
/// - 分頁中途出現「有總筆數但列表為空」的異常狀況。
pub async fn fetch_category_snapshots(
    category: &YahooClassCategory,
) -> Result<CategoryFetchResult> {
    collect_category_pages(
        category,
        |offset| fetch_class_quotes_page(category, offset),
        || sleep(PAGE_REQUEST_INTERVAL),
    )
    .await
}

#[cfg(test)]
mod tests;
