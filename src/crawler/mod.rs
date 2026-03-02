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
//! - **DDNS 支援**：包含自動更新 DDNS (如 Afraid, Dynu, No-IP) 的相關模組。

use std::{
    collections::HashMap,
    sync::{Mutex, atomic::{AtomicUsize, Ordering}},
    time::Instant,
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use rust_decimal::Decimal;

use crate::{
    crawler::{
        cnyes::CnYes,
        cmoney::CMoney,
        fugle::Fugle,
        histock::HiStock,
        megatime::PcHome,
        nstock::NStock,
        yahoo::Yahoo,
        yuanta::Yuanta,
    },
    declare,
    logging,
};

/// 動態 DNS 服務 (Afraid DNS)
pub mod afraid;
/// 臺灣銀行 (提供匯率、財務報表等資料)
pub mod bank_of_taiwan;
/// IP 資訊服務 (BigDataCloud)
pub mod bigdatacloud;
/// 理財寶 - 股市爆料同學會 (提供即時股價與社群資訊)
pub mod cmoney;
/// 鉅亨網 (提供財經新聞與即時報價)
pub mod cnyes;
/// 動態 DNS 服務 (Dynu)
pub mod dynu;
/// 富邦證券
pub mod fbs;
/// Fugle 行情 API
pub mod fugle;
/// Goodinfo! 台灣股市資訊網 (提供股利與基本面資料)
pub mod goodinfo;
/// HiStock 嗨投資 (財經社群與數據站)
pub mod histock;
/// IP 檢測服務 (ipify)
pub mod ipify;
/// IP 資訊查詢 (ipinfo)
pub mod ipinfo;
/// PCHOME 股市 (提供即時行情)
pub mod megatime;
/// MoneyDJ 理財網 (嘉實資訊，提供詳盡的財務指標)
pub mod moneydj;
/// IP 檢測服務 (MyIP)
pub mod myip;
/// 動態 DNS 服務 (No-IP)
pub mod noip;
/// NStock 恩投資 (提供 EPS 與各類統計數據)
pub mod nstock;
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

/// 標記採集站點的全局遊標。
///
/// 為了避免單一站點請求過於頻繁導致被封鎖，系統使用此遊標進行輪詢 (Round-robin)。
/// 每發起一次請求，遊標就會遞增，確保下一次嘗試會從不同的來源開始。
static INDEX: AtomicUsize = AtomicUsize::new(0);
/// 各報價站點的延遲統計（供收盤後輸出人類可讀的追蹤資訊）。
static SITE_LATENCY_STATS: Lazy<Mutex<HashMap<&'static str, SiteLatencyStats>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Default)]
struct SiteLatencyStats {
    durations_ms: Vec<u64>,
}

struct SiteLatencySnapshot {
    site_name: &'static str,
    count: usize,
    avg_ms: u64,
    p99_ms: u64,
}

impl SiteLatencyStats {
    fn record(&mut self, elapsed_ms: u64) {
        self.durations_ms.push(elapsed_ms);
    }

    fn sample_count(&self) -> usize {
        self.durations_ms.len()
    }

    fn average_ms(&self) -> u64 {
        if self.durations_ms.is_empty() {
            return 0;
        }

        let sum: u128 = self.durations_ms.iter().map(|v| u128::from(*v)).sum();
        (sum / self.durations_ms.len() as u128) as u64
    }

    fn p99_ms(&self) -> u64 {
        if self.durations_ms.is_empty() {
            return 0;
        }

        let mut values = self.durations_ms.clone();
        values.sort_unstable();

        let len = values.len();
        let idx = ((len * 99).div_ceil(100)).saturating_sub(1);
        values[idx]
    }
}

/// 獲取當前遊標索引並遞增。
///
/// 使用 `AtomicUsize` 確保在多執行緒環境下的原子性。
///
/// # 參數
/// * `max` - 站點總數，當遊標達到此值時會自動歸零。
fn get_and_increment_index(max: usize) -> usize {
    INDEX
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |val| {
            Some((val + 1) % max)
        })
        .unwrap_or(0)
}

/// 記錄單一站點本次請求耗時。
fn record_site_latency(site_name: &'static str, started_at: Instant) {
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if let Ok(mut stats) = SITE_LATENCY_STATS.lock() {
        stats.entry(site_name).or_default().record(elapsed_ms);
    }
}

/// 輸出站點耗時統計並清空當前累積資料。
///
/// 供收盤事件呼叫，將當日 `fetch_stock_price_from_remote_site` 與
/// `fetch_stock_quotes_from_remote_site` 的站點耗時統一輸出。
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
            site_name: *site_name,
            count: site_stats.sample_count(),
            avg_ms: site_stats.average_ms(),
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
            "站點整體耗時統計 {}: count={}, avg={}ms, p99={}ms",
            entry.site_name, entry.count, entry.avg_ms, entry.p99_ms
        ));
    }

    stats.clear();
}

/// 從多個遠端站點中輪詢獲取股票的最新成交價。
///
/// 此函數會嘗試預設的站點清單，如果某個站點失敗，會自動嘗試下一個，直到成功或所有站點都失敗為止。
/// 支援的站點包括：Yahoo, CMoney, NStock, PcHome, CnYes。
///
/// # 參數
/// * `stock_symbol` - 股票代碼 (例如: "2330")
///
/// # 傳回值
/// 成功時傳回 `Decimal` 型態的股價（已標準化），失敗時傳回錯誤描述。
pub async fn fetch_stock_price_from_remote_site(stock_symbol: &str) -> Result<Decimal> {
    let site_names = [
        "Yahoo", "Fugle", "NStock", "CMoney", "HiStock", "CnYes", "Yuanta", "PcHome",
    ];
    let sites = [
        Yahoo::get_stock_price,
        Fugle::get_stock_price,
        NStock::get_stock_price,
        CMoney::get_stock_price,
        HiStock::get_stock_price,
        CnYes::get_stock_price,
        Yuanta::get_stock_price,
        PcHome::get_stock_price,
    ];
    let site_len = sites.len();
    let mut errors = Vec::with_capacity(site_len);

    for _ in 0..site_len {
        let current_site = get_and_increment_index(site_len);
        let site_name = site_names[current_site];
        let started_at = Instant::now();
        match sites[current_site](stock_symbol).await {
            Ok(price) => {
                record_site_latency(site_name, started_at);
                return Ok(price.normalize());
            }
            Err(why) => {
                record_site_latency(site_name, started_at);
                errors.push(format!("{site_name}: {why}"));
            }
        }
    }

    Err(anyhow!(
        "Failed to fetch stock price({stock_symbol}) from all sites: {}",
        errors.join(" | ")
    ))
}

/// 從多個遠端站點中輪詢獲取股票的完整報價資訊。
///
/// 此函數包含漲跌、漲幅、開盤、最高、最低等詳細資料。
/// 實作機制與 `fetch_stock_price_from_remote_site` 相同，採用自動備援輪詢。
///
/// # 參數
/// * `stock_symbol` - 股票代碼 (例如: "2330")
///
/// # 傳回值
/// 成功時傳回 `declare::StockQuotes` 結構，包含詳細報價，失敗時傳回錯誤。
pub async fn fetch_stock_quotes_from_remote_site(
    stock_symbol: &str,
) -> Result<declare::StockQuotes> {
    let site_names = [
        "Yahoo", "Fugle", "NStock", "CMoney", "HiStock", "CnYes", "Yuanta", "PcHome",
    ];
    let sites = [
        Yahoo::get_stock_quotes,
        Fugle::get_stock_quotes,
        NStock::get_stock_quotes,
        CMoney::get_stock_quotes,
        HiStock::get_stock_quotes,
        CnYes::get_stock_quotes,
        Yuanta::get_stock_quotes,
        PcHome::get_stock_quotes,
    ];
    let site_len = sites.len();
    let mut errors = Vec::with_capacity(site_len);

    for _ in 0..site_len {
        let current_site = get_and_increment_index(site_len);
        let site_name = site_names[current_site];
        let started_at = Instant::now();
        match sites[current_site](stock_symbol).await {
            Ok(quotes) => {
                record_site_latency(site_name, started_at);
                return Ok(quotes);
            }
            Err(why) => {
                record_site_latency(site_name, started_at);
                errors.push(format!("{site_name}: {why}"));
            }
        }
    }

    Err(anyhow!(
        "Failed to fetch stock quotes({stock_symbol}) from all sites: {}",
        errors.join(" | ")
    ))
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[test]
    fn test_site_latency_stats_average_and_p99() {
        let mut stats = SiteLatencyStats::default();

        for elapsed_ms in [10, 20, 30, 40, 50] {
            stats.record(elapsed_ms);
        }

        assert_eq!(stats.sample_count(), 5);
        assert_eq!(stats.average_ms(), 30);
        assert_eq!(stats.p99_ms(), 50);
    }

    #[test]
    fn test_site_latency_stats_p99_with_single_sample() {
        let mut stats = SiteLatencyStats::default();
        stats.record(88);

        assert_eq!(stats.sample_count(), 1);
        assert_eq!(stats.average_ms(), 88);
        assert_eq!(stats.p99_ms(), 88);
    }

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
