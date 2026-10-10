//! # 站點池與股價聚合 (Site Pool)
//!
//! 此子模組集中管理多來源即時股價／完整報價的輪詢 (round-robin) 與備援機制，
//! 並記錄各站點延遲統計供收盤後輸出。對外公開的 `fetch_*` 函式由 `crawler` 模組
//! 重新匯出，呼叫端路徑維持不變。
//!
//! 內容依職責拆成四個檔案：本檔負責輪詢與備援流程及對外 API，
//! [`registry`] 收納站點清單與函式指標包裝，[`latency`] 負責延遲統計，
//! [`health`] 依各站近期成敗與耗時決定嘗試順序（熔斷與慢站降級）。

mod health;
mod latency;
mod registry;

use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

use anyhow::{Result, anyhow};
use rust_decimal::Decimal;

use self::health::{plan_attempt_order, record_site_outcome};
pub use self::latency::flush_site_latency_stats;
use self::latency::record_site_latency;
use self::registry::{
    ALL_PRICE_SITES, ALL_QUOTE_SITES, BACKUP_PRICE_SITES, PriceSite, QuoteSite, Site,
};
use crate::core::{declare, util::http};

/// 標記採集站點的全局遊標。
///
/// 為了避免單一站點請求過於頻繁導致被封鎖，系統使用此遊標進行輪詢 (Round-robin)。
/// 每次抓取只取一次遊標作為起點，再依序走過站點池；不能每試一站就取一次，
/// 否則多檔股票並發抓取時遊標互相穿插，同一檔會重複打同一站、漏掉其他站
/// （2026-10-07 正式機的失敗訊息裡 Fugle 出現 3 次、PcHome 一次都沒試到）。
static INDEX: AtomicUsize = AtomicUsize::new(0);

/// 單次「最新成交價」抓取的結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchedStockPrice {
    /// 標準化後的最新成交價。
    pub price: Decimal,
    /// 實際成功回應的採集站點名稱。
    pub site_name: &'static str,
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

/// 依指定站點池輪詢抓取資料（最新成交價與完整報價共用）。
///
/// # 參數
/// - `stock_symbol`: 股票代號。
/// - `sites`: 要參與輪詢的站點池。
/// - `what`: 錯誤訊息中的資料名稱（`stock price`／`stock quotes`）。
/// - `error_scope`: 錯誤訊息中使用的站點池描述字串。
///
/// # 行為
/// - 依 [`get_and_increment_index`] 取得本輪起始站點，避免所有請求都從同一站開始。
/// - 依 [`plan_attempt_order`] 略過熔斷中的站點、把慢站排到最後，每站最多試一次。
/// - 每站的請求以 [`http::fail_fast`] 執行：不重試、單次逾時 6 秒。
/// - 成功時立即回傳資料與命中的站點名稱。
/// - 失敗時累積各站點錯誤，全部失敗後再整體回傳。
async fn fetch_from_site_pool<T>(
    stock_symbol: &str,
    sites: &[Site<T>],
    what: &str,
    error_scope: &str,
) -> Result<(T, &'static str)> {
    let names = sites.iter().map(|site| site.name).collect::<Vec<_>>();
    let order = plan_attempt_order(&names, get_and_increment_index(sites.len()));
    let mut errors = Vec::with_capacity(order.len());

    for idx in order {
        let site = sites[idx];
        let started_at = Instant::now();
        // 即時抓價失敗就換下一站，不在單一站上做網路重試與 429 backoff。
        let result = http::fail_fast((site.fetch)(stock_symbol)).await;
        record_site_latency(site.name, started_at);
        record_site_outcome(site.name, result.is_ok(), started_at);
        match result {
            Ok(value) => return Ok((value, site.name)),
            Err(why) => errors.push(format!("{}: {why}", site.name)),
        }
    }

    Err(anyhow!(
        "Failed to fetch {what}({stock_symbol}) from {error_scope}: {}",
        errors.join(" | ")
    ))
}

/// 依指定站點池輪詢抓取「最新成交價」，價格標準化後回傳。
async fn fetch_stock_price_from_site_pool(
    stock_symbol: &str,
    sites: &[PriceSite],
    error_scope: &str,
) -> Result<FetchedStockPrice> {
    let (price, site_name) =
        fetch_from_site_pool(stock_symbol, sites, "stock price", error_scope).await?;
    Ok(FetchedStockPrice {
        price: price.normalize(),
        site_name,
    })
}

/// 依指定站點池輪詢抓取「完整報價」。
async fn fetch_stock_quotes_from_site_pool(
    stock_symbol: &str,
    sites: &[QuoteSite],
    error_scope: &str,
) -> Result<declare::StockQuotes> {
    fetch_from_site_pool(stock_symbol, sites, "stock quotes", error_scope)
        .await
        .map(|(quotes, _)| quotes)
}

/// 從多個遠端站點中輪詢獲取股票的最新成交價。
///
/// 此函數會嘗試預設的站點清單，如果某個站點失敗，會自動嘗試下一個，直到成功或所有站點都失敗為止。
/// 支援的站點包括：Yahoo, Fugle, NStock, CMoney, CnYes, PcHome, BigGo。
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
/// 支援的站點包括：Fugle, NStock, CMoney, CnYes, PcHome, BigGo。
/// 實際站點定義在 [`BACKUP_PRICE_SITES`]：比一般路徑少了 Yahoo，原因見該清單說明。
///
/// # 參數
/// * `stock_symbol` - 股票代碼 (例如: "2330")
///
/// # 傳回值
/// 成功時傳回 `Decimal` 型態的股價（已標準化），失敗時傳回錯誤描述。
pub async fn fetch_stock_price_from_backup_sites(stock_symbol: &str) -> Result<Decimal> {
    fetch_stock_price_from_site_pool(stock_symbol, &BACKUP_PRICE_SITES, "backup sites")
        .await
        .map(|result| result.price)
}

/// 從多個備援站點中輪詢獲取股票的最新成交價，並回傳命中的站點名稱。
///
/// 站點池同 [`fetch_stock_price_from_backup_sites`]。
pub async fn fetch_stock_price_from_backup_sites_with_source(
    stock_symbol: &str,
) -> Result<FetchedStockPrice> {
    fetch_stock_price_from_site_pool(stock_symbol, &BACKUP_PRICE_SITES, "backup sites").await
}

/// 從多個遠端站點中輪詢獲取股票的完整報價資訊。
///
/// 此函數包含漲跌、漲幅、開盤、最高、最低等詳細資料。
/// 實作機制與 `fetch_stock_price_from_remote_site` 相同，採用自動備援輪詢。
/// 支援的站點包括：Fugle, NStock, CMoney, CnYes, PcHome。
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
    use super::*;

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

    fn ok_price<'a>(_: &'a str) -> registry::SiteFuture<'a, Decimal> {
        Box::pin(async { Ok(Decimal::new(12340, 2)) })
    }

    fn fail_price<'a>(stock_symbol: &'a str) -> registry::SiteFuture<'a, Decimal> {
        Box::pin(async move { Err(anyhow!("{stock_symbol} 查無報價")) })
    }

    fn ok_quotes<'a>(stock_symbol: &'a str) -> registry::SiteFuture<'a, declare::StockQuotes> {
        Box::pin(async move {
            Ok(declare::StockQuotes {
                stock_symbol: stock_symbol.to_owned(),
                price: 101.5,
                change: 1.5,
                change_range: 1.5,
            })
        })
    }

    fn fail_quotes<'a>(_: &'a str) -> registry::SiteFuture<'a, declare::StockQuotes> {
        Box::pin(async { Err(anyhow!("欄位格式不符")) })
    }

    /// 失敗的站點換下一站；回傳命中的站點並把價格標準化。
    ///
    /// 起始站點由全域遊標決定，不論從哪一站開始，結果都要是成功的那一站。
    /// 站點名稱各測試獨立，避免健康紀錄（熔斷）互相影響。
    #[tokio::test]
    async fn price_pool_falls_back_to_next_site_and_normalizes() {
        let sites = [
            PriceSite {
                name: "TestPriceDown",
                fetch: fail_price,
            },
            PriceSite {
                name: "TestPriceUp",
                fetch: ok_price,
            },
        ];
        for _ in 0..sites.len() {
            let fetched = fetch_stock_price_from_site_pool("2330", &sites, "test pool")
                .await
                .expect("有一站成功就要回傳");
            assert_eq!(fetched.site_name, "TestPriceUp");
            assert_eq!(fetched.price.to_string(), "123.4");
        }
    }

    /// 全部站點失敗時，錯誤訊息列出每一站的原因。
    #[tokio::test]
    async fn price_pool_reports_every_site_when_all_fail() {
        let sites = [
            PriceSite {
                name: "TestPriceDownA",
                fetch: fail_price,
            },
            PriceSite {
                name: "TestPriceDownB",
                fetch: fail_price,
            },
        ];
        let message = fetch_stock_price_from_site_pool("2330", &sites, "test pool")
            .await
            .expect_err("全部失敗要回錯誤")
            .to_string();
        assert!(
            message.starts_with("Failed to fetch stock price(2330) from test pool: "),
            "{message}"
        );
        assert!(
            message.contains("TestPriceDownA: 2330 查無報價"),
            "{message}"
        );
        assert!(
            message.contains("TestPriceDownB: 2330 查無報價"),
            "{message}"
        );
    }

    /// 完整報價走同一套輪詢；全部失敗時錯誤訊息標明是 stock quotes。
    #[tokio::test]
    async fn quote_pool_falls_back_and_reports_failures() {
        let sites = [
            QuoteSite {
                name: "TestQuoteDown",
                fetch: fail_quotes,
            },
            QuoteSite {
                name: "TestQuoteUp",
                fetch: ok_quotes,
            },
        ];
        let quotes = fetch_stock_quotes_from_site_pool("1101", &sites, "test pool")
            .await
            .expect("有一站成功就要回傳");
        assert_eq!(quotes.stock_symbol, "1101");
        assert_eq!(quotes.price, 101.5);

        let failing = [QuoteSite {
            name: "TestQuoteDownOnly",
            fetch: fail_quotes,
        }];
        let message = fetch_stock_quotes_from_site_pool("1101", &failing, "test pool")
            .await
            .expect_err("全部失敗要回錯誤")
            .to_string();
        assert_eq!(
            message,
            "Failed to fetch stock quotes(1101) from test pool: TestQuoteDownOnly: 欄位格式不符"
        );
    }

    /// 驗證完整站點池可以成功抓取多檔股票的最新成交價。
    ///
    /// 此測試會實際連線外部站點，主要用於手動驗證輪詢與備援流程。
    #[tokio::test]
    #[ignore]
    async fn test_fetch_stock_price_from_remote_site() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 fetch_price");

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
                    tracing::debug!("Failed to fetch_price because {:?}", why);
                }
            }
        }
        flush_site_latency_stats();
        tracing::debug!("結束 fetch_price");
    }

    /// 驗證完整站點池可以成功抓取多檔股票的完整報價資訊。
    ///
    /// 此測試會實際連線外部站點，主要用於手動驗證完整報價輪詢流程。
    #[tokio::test]
    #[ignore]
    async fn test_fetch_stock_quotes_from_remote_site() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 fetch_stock_quotes_from_remote_site");

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
                    tracing::debug!("Failed to fetch_price because {:?}", why);
                }
            }
        }

        tracing::debug!("結束 fetch_stock_quotes_from_remote_site");
    }
}
