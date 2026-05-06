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

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex, OnceLock,
    },
    time::Instant,
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use rust_decimal::Decimal;

use crate::{
    crawler::{
        cmoney::CMoney, cnyes::CnYes, fugle::Fugle, megatime::PcHome, nstock::NStock,
        winvest::Winvest, yahoo::Yahoo,
    },
    declare, logging, util,
};

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
pub(super) mod share;
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
    logging::debug_file_async("開始 get_stock_price".to_string());

    match S::get_stock_price(stock_symbol).await {
        Ok(price) => {
            dbg!(&price);
            logging::debug_file_async(format!("price : {:#?}", price));
        }
        Err(why) => {
            logging::debug_file_async(format!("Failed to get_stock_price because {:?}", why));
        }
    }

    logging::debug_file_async("結束 get_stock_price".to_string());
}

#[cfg(test)]
pub(crate) async fn log_public_ip_visit_test<F, Fut>(visit: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<String>>,
{
    match visit().await {
        Ok(ip) => {
            dbg!(ip);
        }
        Err(why) => {
            logging::error_file_async(format!("Failed to get because {:?}", why));
        }
    }
}

/// 讀取回傳純文字 IP 的 public IP endpoint。
pub(crate) async fn get_public_ip_text(
    url_cache: &OnceLock<String>,
    host: &str,
    path: &str,
    trim: bool,
) -> Result<String> {
    let url = url_cache.get_or_init(|| format!("https://{host}{path}"));
    let ip = util::http::get(url, None).await?;

    if trim {
        Ok(ip.trim().to_string())
    } else {
        Ok(ip)
    }
}

/// 標記採集站點的全局遊標。
///
/// 為了避免單一站點請求過於頻繁導致被封鎖，系統使用此遊標進行輪詢 (Round-robin)。
/// 每發起一次請求，遊標就會遞增，確保下一次嘗試會從不同的來源開始。
static INDEX: AtomicUsize = AtomicUsize::new(0);
/// 各報價站點的延遲統計（供收盤後輸出人類可讀的追蹤資訊）。
static SITE_LATENCY_STATS: Lazy<Mutex<HashMap<&'static str, SiteLatencyStats>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// 「最新成交價」非同步抓取函式的 boxed future 型別。
///
/// 這個型別別名用來收斂各站點 `async_trait` 產生出的回傳型別，
/// 讓站點池可以用一致的函式指標簽名儲存不同來源。
type StockPriceFuture<'a> = Pin<Box<dyn Future<Output = Result<Decimal>> + Send + 'a>>;
/// 「完整報價」非同步抓取函式的 boxed future 型別。
///
/// 用途與 [`StockPriceFuture`] 相同，只是回傳內容改為 [`declare::StockQuotes`]。
type StockQuotesFuture<'a> =
    Pin<Box<dyn Future<Output = Result<declare::StockQuotes>> + Send + 'a>>;
/// 「最新成交價」站點 wrapper 函式的統一函式指標型別。
type StockPriceFetcher = for<'a> fn(&'a str) -> StockPriceFuture<'a>;
/// 「完整報價」站點 wrapper 函式的統一函式指標型別。
type StockQuotesFetcher = for<'a> fn(&'a str) -> StockQuotesFuture<'a>;

/// 單一「股價」站點的描述。
///
/// 將站點名稱與對應抓價函式綁在一起，避免名稱陣列與函式陣列分離後產生順序錯位。
#[derive(Clone, Copy)]
struct PriceSite {
    name: &'static str,
    fetch: StockPriceFetcher,
}

/// 單一「完整報價」站點的描述。
///
/// 結構與 [`PriceSite`] 相同，但抓取的是開高低收、漲跌幅等完整報價資料。
#[derive(Clone, Copy)]
struct QuoteSite {
    name: &'static str,
    fetch: StockQuotesFetcher,
}

/// 單次「最新成交價」抓取的結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchedStockPrice {
    /// 標準化後的最新成交價。
    pub price: Decimal,
    /// 實際成功回應的採集站點名稱。
    pub site_name: &'static str,
}

/// 產生「最新成交價」wrapper 函式。
///
/// `async_trait` 產生的關聯函式型別，無法直接穩定地放進模組層級常數陣列；
/// 因此透過此巨集產生一層薄 wrapper，讓站點池可以持有一致的函式指標型別。
///
/// # 使用方式
/// ```ignore
/// define_stock_price_fetcher!(
///     "將 FooSite 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
///     fetch_foosite_price,
///     FooSite
/// );
/// ```
macro_rules! define_stock_price_fetcher {
    ($doc:literal, $fn_name:ident, $site:ty) => {
        #[doc = $doc]
        fn $fn_name<'a>(stock_symbol: &'a str) -> StockPriceFuture<'a> {
            <$site as StockInfo>::get_stock_price(stock_symbol)
        }
    };
}

/// 產生「完整報價」wrapper 函式。
///
/// 用途與 [`define_stock_price_fetcher`] 相同，只是包裝的是
/// `StockInfo::get_stock_quotes`，供完整報價站點池重複使用。
///
/// # 使用方式
/// ```ignore
/// define_stock_quotes_fetcher!(
///     "將 FooSite 的 `StockInfo::get_stock_quotes` 包裝成可放入完整報價站點池的函式指標。",
///     fetch_foosite_quotes,
///     FooSite
/// );
/// ```
macro_rules! define_stock_quotes_fetcher {
    ($doc:literal, $fn_name:ident, $site:ty) => {
        #[doc = $doc]
        fn $fn_name<'a>(stock_symbol: &'a str) -> StockQuotesFuture<'a> {
            <$site as StockInfo>::get_stock_quotes(stock_symbol)
        }
    };
}

define_stock_price_fetcher!(
    "將 Yahoo 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
    fetch_yahoo_price,
    Yahoo
);
define_stock_price_fetcher!(
    "將 Fugle 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
    fetch_fugle_price,
    Fugle
);
define_stock_price_fetcher!(
    "將 NStock 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
    fetch_nstock_price,
    NStock
);
define_stock_price_fetcher!(
    "將 CMoney 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
    fetch_cmoney_price,
    CMoney
);
define_stock_price_fetcher!(
    "將 CnYes 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
    fetch_cnyes_price,
    CnYes
);
define_stock_price_fetcher!(
    "將 PcHome 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
    fetch_pchome_price,
    PcHome
);
define_stock_price_fetcher!(
    "將 Winvest 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
    fetch_winvest_price,
    Winvest
);

define_stock_quotes_fetcher!(
    "將 Fugle 的 `StockInfo::get_stock_quotes` 包裝成可放入完整報價站點池的函式指標。",
    fetch_fugle_quotes,
    Fugle
);
define_stock_quotes_fetcher!(
    "將 NStock 的 `StockInfo::get_stock_quotes` 包裝成可放入完整報價站點池的函式指標。",
    fetch_nstock_quotes,
    NStock
);
define_stock_quotes_fetcher!(
    "將 CMoney 的 `StockInfo::get_stock_quotes` 包裝成可放入完整報價站點池的函式指標。",
    fetch_cmoney_quotes,
    CMoney
);
define_stock_quotes_fetcher!(
    "將 CnYes 的 `StockInfo::get_stock_quotes` 包裝成可放入完整報價站點池的函式指標。",
    fetch_cnyes_quotes,
    CnYes
);
define_stock_quotes_fetcher!(
    "將 PcHome 的 `StockInfo::get_stock_quotes` 包裝成可放入完整報價站點池的函式指標。",
    fetch_pchome_quotes,
    PcHome
);
define_stock_quotes_fetcher!(
    "將 Winvest 的 `StockInfo::get_stock_quotes` 包裝成可放入完整報價站點池的函式指標。",
    fetch_winvest_quotes,
    Winvest
);

/// 所有可用的「最新成交價」站點池。
///
/// 此順序同時代表 round-robin 的輪詢候選順序。
/// `HiStock` 已從此站點池移除，因為目前即時股價採集改由它自己的背景排程負責。
///
/// 目前站點順序如下：
/// - `Yahoo`
/// - `Fugle`
/// - `NStock`
/// - `CMoney`
/// - `CnYes`
/// - `PcHome`
/// - `Winvest`
///
/// `Yuanta` 已從此站點池移除，因為其資料目前觀察到為前一交易日資料，
/// 不符合即時追蹤用途。
const ALL_PRICE_SITES: [PriceSite; 7] = [
    PriceSite {
        name: "Yahoo",
        fetch: fetch_yahoo_price,
    },
    PriceSite {
        name: "Fugle",
        fetch: fetch_fugle_price,
    },
    PriceSite {
        name: "NStock",
        fetch: fetch_nstock_price,
    },
    PriceSite {
        name: "CMoney",
        fetch: fetch_cmoney_price,
    },
    PriceSite {
        name: "CnYes",
        fetch: fetch_cnyes_price,
    },
    PriceSite {
        name: "PcHome",
        fetch: fetch_pchome_price,
    },
    PriceSite {
        name: "Winvest",
        fetch: fetch_winvest_price,
    },
];

/// 所有可用的「完整報價」站點池。
///
/// 這條路徑只保留目前仍用於單股完整報價備援的站點，
/// `HiStock` 也已改由它自己的背景排程負責，不再納入此站點池。
///
/// 目前站點順序如下：
/// - `Fugle`
/// - `NStock`
/// - `CMoney`
/// - `CnYes`
/// - `PcHome`
/// - `Winvest`
///
/// `Yuanta` 已從此站點池移除，因為其資料目前觀察到為前一交易日資料，
/// 不適合用作即時完整報價來源。
const ALL_QUOTE_SITES: [QuoteSite; 6] = [
    QuoteSite {
        name: "Fugle",
        fetch: fetch_fugle_quotes,
    },
    QuoteSite {
        name: "NStock",
        fetch: fetch_nstock_quotes,
    },
    QuoteSite {
        name: "CMoney",
        fetch: fetch_cmoney_quotes,
    },
    QuoteSite {
        name: "CnYes",
        fetch: fetch_cnyes_quotes,
    },
    QuoteSite {
        name: "PcHome",
        fetch: fetch_pchome_quotes,
    },
    QuoteSite {
        name: "Winvest",
        fetch: fetch_winvest_quotes,
    },
];

/// 單一站點在目前統計期間內累積的延遲樣本。
#[derive(Default)]
struct SiteLatencyStats {
    /// 每次請求的耗時，單位為毫秒。
    durations_ms: Vec<u64>,
}

/// 單一站點延遲統計的彙總快照。
///
/// 此結構只在輸出 log 前短暫建立，用來承載排序後的摘要資訊。
struct SiteLatencySnapshot {
    /// 站點名稱。
    site_name: &'static str,
    /// 取樣次數。
    count: usize,
    /// 平均延遲，單位為毫秒。
    avg_ms: u64,
    /// 第 50 百分位延遲，單位為毫秒。
    p50_ms: u64,
    /// 第 70 百分位延遲，單位為毫秒。
    p70_ms: u64,
    /// 第 99 百分位延遲，單位為毫秒。
    p99_ms: u64,
}

impl SiteLatencyStats {
    /// 新增一筆站點延遲樣本。
    fn record(&mut self, elapsed_ms: u64) {
        self.durations_ms.push(elapsed_ms);
    }

    /// 取得目前累積的樣本數。
    fn sample_count(&self) -> usize {
        self.durations_ms.len()
    }

    /// 計算平均延遲，單位為毫秒。
    fn average_ms(&self) -> u64 {
        if self.durations_ms.is_empty() {
            return 0;
        }

        let sum: u128 = self.durations_ms.iter().map(|v| u128::from(*v)).sum();
        (sum / self.durations_ms.len() as u128) as u64
    }

    /// 計算指定百分位延遲，單位為毫秒。
    ///
    /// # 參數
    /// - `percentile`: 百分位數，範圍應介於 `1..=100`。
    ///
    /// 若樣本不足對應百分位所需數量，會回傳排序後最接近該百分位的樣本值。
    fn percentile_ms(&self, percentile: usize) -> u64 {
        if self.durations_ms.is_empty() {
            return 0;
        }

        let mut values = self.durations_ms.clone();
        values.sort_unstable();

        let len = values.len();
        let percentile = percentile.clamp(1, 100);
        let idx = (len * percentile).div_ceil(100).saturating_sub(1);
        values[idx]
    }

    /// 計算第 50 百分位延遲，單位為毫秒。
    fn p50_ms(&self) -> u64 {
        self.percentile_ms(50)
    }

    /// 計算第 70 百分位延遲，單位為毫秒。
    fn p70_ms(&self) -> u64 {
        self.percentile_ms(70)
    }

    /// 計算第 99 百分位延遲，單位為毫秒。
    ///
    /// 若樣本不足 100 筆，會回傳排序後靠近尾端的樣本值，
    /// 作為保守的高延遲觀察指標。
    fn p99_ms(&self) -> u64 {
        self.percentile_ms(99)
    }
}

/// 獲取當前遊標索引並遞增。
///
/// 使用 `AtomicUsize` 確保在多執行緒環境下的原子性。
///
/// # 參數
/// * `max` - 站點總數，當遊標達到此值時會自動歸零。
fn get_and_increment_index(max: usize) -> usize {
    INDEX.fetch_add(1, Ordering::SeqCst) % max
}

/// 記錄單一站點本次請求耗時。
fn record_site_latency(site_name: &'static str, started_at: Instant) {
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if let Ok(mut stats) = SITE_LATENCY_STATS.lock() {
        stats.entry(site_name).or_default().record(elapsed_ms);
    }
}

/// 依指定站點池輪詢抓取「最新成交價」。
///
/// # 參數
/// - `stock_symbol`: 股票代號。
/// - `sites`: 要參與輪詢的站點池。
/// - `error_scope`: 錯誤訊息中使用的站點池描述字串。
///
/// # 行為
/// - 依 [`get_and_increment_index`] 取得本輪起始站點，避免所有請求都從同一站開始。
/// - 成功時立即回傳標準化後的股價。
/// - 失敗時累積各站點錯誤，全部失敗後再整體回傳。
async fn fetch_stock_price_from_site_pool(
    stock_symbol: &str,
    sites: &[PriceSite],
    error_scope: &str,
) -> Result<FetchedStockPrice> {
    let site_len = sites.len();
    let mut errors = Vec::with_capacity(site_len);

    for _ in 0..site_len {
        let current_site = get_and_increment_index(site_len);
        let site = sites[current_site];
        let started_at = Instant::now();
        match (site.fetch)(stock_symbol).await {
            Ok(price) => {
                record_site_latency(site.name, started_at);
                return Ok(FetchedStockPrice {
                    price: price.normalize(),
                    site_name: site.name,
                });
            }
            Err(why) => {
                record_site_latency(site.name, started_at);
                errors.push(format!("{}: {why}", site.name));
            }
        }
    }

    Err(anyhow!(
        "Failed to fetch stock price({stock_symbol}) from {error_scope}: {}",
        errors.join(" | ")
    ))
}

/// 依指定站點池輪詢抓取「完整報價」。
///
/// # 參數
/// - `stock_symbol`: 股票代號。
/// - `sites`: 要參與輪詢的站點池。
/// - `error_scope`: 錯誤訊息中使用的站點池描述字串。
///
/// # 行為
/// - 與 [`fetch_stock_price_from_site_pool`] 相同，差別只在回傳型別為完整報價。
async fn fetch_stock_quotes_from_site_pool(
    stock_symbol: &str,
    sites: &[QuoteSite],
    error_scope: &str,
) -> Result<declare::StockQuotes> {
    let site_len = sites.len();
    let mut errors = Vec::with_capacity(site_len);

    for _ in 0..site_len {
        let current_site = get_and_increment_index(site_len);
        let site = sites[current_site];
        let started_at = Instant::now();
        match (site.fetch)(stock_symbol).await {
            Ok(quotes) => {
                record_site_latency(site.name, started_at);
                return Ok(quotes);
            }
            Err(why) => {
                record_site_latency(site.name, started_at);
                errors.push(format!("{}: {why}", site.name));
            }
        }
    }

    Err(anyhow!(
        "Failed to fetch stock quotes({stock_symbol}) from {error_scope}: {}",
        errors.join(" | ")
    ))
}

/// 輸出站點耗時統計並清空當前累積資料。
///
/// 供收盤事件呼叫，將當日 `fetch_stock_price_from_remote_site` 與
/// `fetch_stock_quotes_from_remote_site` 的站點耗時統一輸出。
/// 摘要欄位包含取樣次數、平均耗時，以及 `p50`、`p70`、`p99` 百分位延遲。
pub fn flush_site_latency_stats() {
    let mut stats = match SITE_LATENCY_STATS.lock() {
        Ok(guard) => guard,
        Err(_) => {
            logging::error_file_async("Failed to lock site latency stats for flush");
            return;
        }
    };

    if stats.is_empty() {
        logging::info_file_async("站點延遲統計: 無資料");
        return;
    }

    let mut entries = stats
        .iter()
        .map(|(site_name, site_stats)| SiteLatencySnapshot {
            site_name,
            count: site_stats.sample_count(),
            avg_ms: site_stats.average_ms(),
            p50_ms: site_stats.p50_ms(),
            p70_ms: site_stats.p70_ms(),
            p99_ms: site_stats.p99_ms(),
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.p99_ms
            .cmp(&right.p99_ms)
            .then(left.avg_ms.cmp(&right.avg_ms))
            .then(left.site_name.cmp(right.site_name))
    });

    for entry in entries {
        logging::info_file_async(format!(
            "站點整體耗時統計 {}: count={}, avg={}ms, p50={}ms, p70={}ms, p99={}ms",
            entry.site_name, entry.count, entry.avg_ms, entry.p50_ms, entry.p70_ms, entry.p99_ms
        ));
    }

    stats.clear();
}

/// 從多個遠端站點中輪詢獲取股票的最新成交價。
///
/// 此函數會嘗試預設的站點清單，如果某個站點失敗，會自動嘗試下一個，直到成功或所有站點都失敗為止。
/// 支援的站點包括：Yahoo, Fugle, NStock, CMoney, CnYes, PcHome, Winvest。
/// 實際站點定義集中在 [`ALL_PRICE_SITES`]。
/// 此函式不再經過 `HiStock`。
///
/// # 參數
/// * `stock_symbol` - 股票代碼 (例如: "2330")
///
/// # 傳回值
/// 成功時傳回 `Decimal` 型態的股價（已標準化），失敗時傳回錯誤描述。
pub async fn fetch_stock_price_from_remote_site(stock_symbol: &str) -> Result<Decimal> {
    fetch_stock_price_from_site_pool(stock_symbol, &ALL_PRICE_SITES, "all sites")
        .await
        .map(|result| result.price)
}

/// 從多個遠端站點中輪詢獲取股票的最新成交價，但排除 HiStock。
///
/// 此函數主要用於 HiStock 已有獨立背景排程時的備援抓價情境，
/// 避免同一支股票同時由兩套流程對 HiStock 重複請求。
///
/// 支援的站點包括：Yahoo, Fugle, NStock, CMoney, CnYes, PcHome, Winvest。
/// 實際站點定義直接重用 [`ALL_PRICE_SITES`]。
/// 也就是說，最新成交價的一般抓價路徑與備援抓價路徑目前使用相同站點集合。
///
/// # 參數
/// * `stock_symbol` - 股票代碼 (例如: "2330")
///
/// # 傳回值
/// 成功時傳回 `Decimal` 型態的股價（已標準化），失敗時傳回錯誤描述。
pub async fn fetch_stock_price_from_backup_sites(stock_symbol: &str) -> Result<Decimal> {
    fetch_stock_price_from_site_pool(stock_symbol, &ALL_PRICE_SITES, "backup sites")
        .await
        .map(|result| result.price)
}

/// 從多個備援站點中輪詢獲取股票的最新成交價，並回傳命中的站點名稱。
pub async fn fetch_stock_price_from_backup_sites_with_source(
    stock_symbol: &str,
) -> Result<FetchedStockPrice> {
    fetch_stock_price_from_site_pool(stock_symbol, &ALL_PRICE_SITES, "backup sites").await
}

/// 從多個遠端站點中輪詢獲取股票的完整報價資訊。
///
/// 此函數包含漲跌、漲幅、開盤、最高、最低等詳細資料。
/// 實作機制與 `fetch_stock_price_from_remote_site` 相同，採用自動備援輪詢。
/// 支援的站點包括：Fugle, NStock, CMoney, CnYes, PcHome, Winvest。
/// 實際站點定義集中在 [`ALL_QUOTE_SITES`]。
///
/// # 參數
/// * `stock_symbol` - 股票代碼 (例如: "2330")
///
/// # 傳回值
/// 成功時傳回 `declare::StockQuotes` 結構，包含詳細報價，失敗時傳回錯誤。
pub async fn fetch_stock_quotes_from_remote_site(
    stock_symbol: &str,
) -> Result<declare::StockQuotes> {
    fetch_stock_quotes_from_site_pool(stock_symbol, &ALL_QUOTE_SITES, "all sites").await
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    /// 驗證站點延遲統計可以正確計算平均值與各主要百分位。
    #[test]
    fn test_site_latency_stats_average_and_percentiles() {
        let mut stats = SiteLatencyStats::default();

        for elapsed_ms in [10, 20, 30, 40, 50] {
            stats.record(elapsed_ms);
        }

        assert_eq!(stats.sample_count(), 5);
        assert_eq!(stats.average_ms(), 30);
        assert_eq!(stats.p50_ms(), 30);
        assert_eq!(stats.p70_ms(), 40);
        assert_eq!(stats.p99_ms(), 50);
    }

    /// 驗證站點延遲統計在單一樣本時仍能正確回傳平均值與各主要百分位。
    #[test]
    fn test_site_latency_stats_percentiles_with_single_sample() {
        let mut stats = SiteLatencyStats::default();
        stats.record(88);

        assert_eq!(stats.sample_count(), 1);
        assert_eq!(stats.average_ms(), 88);
        assert_eq!(stats.p50_ms(), 88);
        assert_eq!(stats.p70_ms(), 88);
        assert_eq!(stats.p99_ms(), 88);
    }

    /// 驗證全域輪詢游標在不同站點池大小間切換時不會產生越界索引。
    #[test]
    fn test_get_and_increment_index_supports_different_pool_sizes() {
        INDEX.store(0, Ordering::SeqCst);

        assert_eq!(get_and_increment_index(9), 0);
        assert_eq!(get_and_increment_index(8), 1);

        INDEX.store(8, Ordering::SeqCst);
        assert_eq!(get_and_increment_index(8), 0);
        assert_eq!(get_and_increment_index(9), 0);
    }

    /// 驗證輸出延遲統計後，累積中的站點資料會被清空。
    #[test]
    fn test_flush_site_latency_stats_clears_data() {
        {
            let mut all_stats = SITE_LATENCY_STATS.lock().expect("lock site latency stats");
            all_stats.clear();
            all_stats.entry("Yahoo").or_default().record(12);
            all_stats.entry("Fugle").or_default().record(34);
        }

        flush_site_latency_stats();

        let all_stats = SITE_LATENCY_STATS.lock().expect("lock site latency stats");
        assert!(all_stats.is_empty());
    }

    /// 驗證完整站點池可以成功抓取多檔股票的最新成交價。
    ///
    /// 此測試會實際連線外部站點，主要用於手動驗證輪詢與備援流程。
    #[tokio::test]
    async fn test_fetch_stock_price_from_remote_site() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_price".to_string());

        let sites = [
            "2330", "1101", "1232", "1303", "1326", "3008", "9941", "2912",
        ];

        for site in sites {
            match fetch_stock_price_from_remote_site(site).await {
                Ok(e) => {
                    //dbg!(e);
                    println!("{}:{}", site, e);
                }
                Err(why) => {
                    logging::debug_file_async(format!("Failed to fetch_price because {:?}", why));
                }
            }
        }
        flush_site_latency_stats();
        logging::debug_file_async("結束 fetch_price".to_string());
    }

    /// 驗證完整站點池可以成功抓取多檔股票的完整報價資訊。
    ///
    /// 此測試會實際連線外部站點，主要用於手動驗證完整報價輪詢流程。
    #[tokio::test]
    async fn test_fetch_stock_quotes_from_remote_site() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_stock_quotes_from_remote_site".to_string());

        let sites = [
            "2330", "1101", "1232", "1303", "1326", "3008", "9941", "2912",
        ];

        for site in sites {
            match fetch_stock_quotes_from_remote_site(site).await {
                Ok(e) => {
                    //dbg!(e);
                    println!("{}:{:?}", site, e);
                }
                Err(why) => {
                    logging::debug_file_async(format!("Failed to fetch_price because {:?}", why));
                }
            }
        }

        logging::debug_file_async("結束 fetch_stock_quotes_from_remote_site".to_string());
    }
}
