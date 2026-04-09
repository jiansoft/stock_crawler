use std::{
    net::IpAddr,
    sync::atomic::{AtomicUsize, Ordering},
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use scraper::{ElementRef, Html, Selector};

use crate::crawler::{bigdatacloud, myip};
use crate::{
    crawler::{ipconfig, ipify, ipinfo, seeip},
    database::table,
    util::{self, map::Keyable, text},
};

/// 台灣 ETF 資訊載體。
///
/// 此結構用於存儲從 TWSE 或 TPEx 採集到的 ETF 基本資料。
#[derive(Debug, Clone)]
pub struct EtfInfo {
    /// 股票代號（例如："0050"）。
    pub stock_symbol: String,
    /// 股票名稱（例如："元大台灣50"）。
    pub name: String,
    /// 上市日期（格式：YYYY-MM-DD）。
    pub listing_date: String,
    /// 產業分類名稱（ETF 固定為 "ETF"）。
    pub industry: String,
    /// 交易市場資料（包含市場 ID、交易所名稱等）。
    pub exchange_market: table::stock_exchange_market::StockExchangeMarket,
    /// 產業分類 ID（專案中 ETF 的固定 ID 是 9001）。
    pub industry_id: i32,
}

/// 年度財報
#[derive(Debug, Clone, PartialEq)]
pub struct AnnualProfit {
    /// Security code
    pub stock_symbol: String,
    /// 財報年度 (Year)
    pub year: i32,
    /// 每股營收
    pub sales_per_share: Decimal,
    /// 每股稅後淨利
    pub earnings_per_share: Decimal,
    /// 每股稅前淨利
    pub profit_before_tax: Decimal,
}

impl AnnualProfit {
    pub fn new(stock_symbol: String) -> Self {
        Self {
            stock_symbol,
            year: 0,
            sales_per_share: Default::default(),
            earnings_per_share: Default::default(),
            profit_before_tax: Default::default(),
        }
    }
}

impl Keyable for AnnualProfit {
    fn key(&self) -> String {
        format!("{}-{}", self.stock_symbol, self.year)
    }

    fn key_with_prefix(&self) -> String {
        format!("AnnualProfit:{}", self.key())
    }
}

#[async_trait]
pub trait AnnualProfitFetcher {
    async fn visit(stock_symbol: &str) -> Result<Vec<AnnualProfit>>;
}

pub(super) async fn fetch_annual_profits(
    url: &str,
    stock_symbol: &str,
) -> Result<Vec<AnnualProfit>> {
    let text = util::http::get(url, None).await?;
    let document = Html::parse_document(&text);
    let selector = Selector::parse("#oMainTable > tbody > tr:nth-child(n+4)")
        .map_err(|why| anyhow!("Failed to Selector::parse because: {:?}", why))?;
    let mut result: Vec<AnnualProfit> = Vec::with_capacity(24);

    for node in document.select(&selector) {
        if let Some(ap) = parse_annual_profit(node, stock_symbol) {
            result.push(ap);
        }
    }

    Ok(result)
}

fn parse_annual_profit(node: ElementRef, stock_symbol: &str) -> Option<AnnualProfit> {
    let tds: Vec<&str> = node.text().map(str::trim).collect();

    if tds.len() < 8 {
        return None;
    }

    let year = text::parse_i32(tds.first()?, None)
        .ok()
        .map(util::datetime::roc_year_to_gregorian_year)?;
    let earnings_per_share = text::parse_decimal(tds.get(7)?, None).ok()?;
    let profit_before_tax = text::parse_decimal(tds.get(6)?, None).unwrap_or(Decimal::ZERO);
    let sales_per_share = text::parse_decimal(tds.get(5)?, None).unwrap_or(Decimal::ZERO);

    Some(AnnualProfit {
        stock_symbol: stock_symbol.to_string(),
        earnings_per_share,
        profit_before_tax,
        sales_per_share,
        year,
    })
}

/// 全域 IP 查詢游標，用於順序輪詢不同的檢測服務。
static IP_INDEX: AtomicUsize = AtomicUsize::new(0);

/// 獲取系統對外的公網 IP 地址。
///
/// 此函式透過多個第三方 IP 檢測服務進行輪詢，以確保在單一服務失效時仍能取得 IP。
/// 為了平衡負載並避免單一服務請求過於頻繁，採用順序輪詢 (Round-robin) 機制切換不同站點。
///
/// # 支援的服務站點
/// - `ipify.org`
/// - `ipconfig.io`
/// - `ipinfo.io`
/// - `seeip.org`
/// - `myip.com`
/// - `bigdatacloud.com`
///
/// # 回傳值
/// - `Ok(String)`: 成功取得、且已正規化的公網 IP 字串。
/// - `Err`: 當所有嘗試的站點均失效時，回傳包含詳細錯誤原因的描述。
pub async fn get_public_ip() -> Result<String> {
    const SERVICES: &[&str] = &[
        "ipify",
        "ipconfig",
        "ipinfo",
        "seeip",
        "myip",
        "bigdatacloud",
    ];

    let mut errors = Vec::with_capacity(SERVICES.len());

    for _ in 0..SERVICES.len() {
        let idx = IP_INDEX.fetch_add(1, Ordering::SeqCst) % SERVICES.len();
        let service_name = SERVICES[idx];

        let result = match service_name {
            "ipify" => ipify::visit().await,
            "ipconfig" => ipconfig::visit().await,
            "ipinfo" => ipinfo::visit().await,
            "seeip" => seeip::visit().await,
            "myip" => myip::visit().await,
            "bigdatacloud" => bigdatacloud::visit().await,
            _ => unreachable!(),
        };

        if let Some(ip) = collect_public_ip_result(service_name, result, &mut errors) {
            return Ok(ip);
        }
    }

    Err(anyhow!(
        "Failed to get public IP from all services: {}",
        errors.join(" | ")
    ))
}

/// 處理單一 IP 來源的回應結果。
///
/// 成功時會回傳已正規化的 IP；失敗時則把錯誤訊息附加到 `errors`，
/// 讓 `get_public_ip()` 最後能一次回報所有來源的失敗原因。
fn collect_public_ip_result(
    service_name: &str,
    result: Result<String>,
    errors: &mut Vec<String>,
) -> Option<String> {
    match result {
        Ok(ip) => match normalize_public_ip(service_name, &ip) {
            Ok(ip) => Some(ip),
            Err(why) => {
                errors.push(why.to_string());
                None
            }
        },
        Err(why) => {
            errors.push(format!("{service_name}: {why}"));
            None
        }
    }
}

/// 將第三方服務回傳的 IP 文字正規化成穩定格式。
///
/// 這裡會先去除前後空白，再要求內容必須能被解析為合法的
/// [`IpAddr`]；若解析成功，會回傳 `IpAddr::to_string()` 的標準化結果。
fn normalize_public_ip(service_name: &str, ip: &str) -> Result<String> {
    let normalized = ip.trim();

    if normalized.is_empty() {
        return Err(anyhow!("{service_name}: empty response"));
    }

    normalized
        .parse::<IpAddr>()
        .map(|ip| ip.to_string())
        .map_err(|why| anyhow!("{service_name}: invalid ip response `{normalized}` because {why}"))
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use anyhow::anyhow;

    use super::{collect_public_ip_result, get_public_ip, normalize_public_ip};

    /// 驗證純文字 IP 即使夾帶換行，也會先 trim 再正常回傳。
    #[test]
    fn test_normalize_public_ip_trims_and_accepts_ipv4() {
        let ip = normalize_public_ip("ipify", " 1.2.3.4\r\n").unwrap();

        assert_eq!(ip, "1.2.3.4");
    }

    /// 驗證 IPv6 內容也可通過驗證並回傳標準格式。
    #[test]
    fn test_normalize_public_ip_accepts_ipv6() {
        let ip = normalize_public_ip("ipinfo", "2001:db8::1").unwrap();

        assert_eq!(ip, "2001:db8::1");
    }

    /// 驗證空白或空字串不會被誤判成成功的 IP 回應。
    #[test]
    fn test_normalize_public_ip_rejects_empty_response() {
        let err = normalize_public_ip("seeip", " \n ").unwrap_err();

        assert!(err.to_string().contains("empty response"));
    }

    /// 驗證錯頁或其他非 IP 內容會被擋下，不會直接流進 DDNS 流程。
    #[test]
    fn test_normalize_public_ip_rejects_non_ip_body() {
        let err = normalize_public_ip("seeip", "<html>rate limited</html>").unwrap_err();

        assert!(err.to_string().contains("invalid ip response"));
    }

    /// 驗證 `get_public_ip` 的單一來源處理邏輯會接受可 trim 的合法 IPv4。
    #[test]
    fn test_get_public_ip_collects_trimmed_ipv4_response() {
        let mut errors = Vec::new();

        let ip = collect_public_ip_result("ipify", Ok(" 1.2.3.4\r\n".to_string()), &mut errors);

        assert_eq!(ip, Some("1.2.3.4".to_string()));
        assert!(errors.is_empty());
    }

    /// 驗證 `get_public_ip` 的單一來源處理邏輯會把非 IP 內容記成錯誤。
    #[test]
    fn test_get_public_ip_collects_invalid_body_as_error() {
        let mut errors = Vec::new();

        let ip = collect_public_ip_result(
            "ipinfo",
            Ok("<html>challenge</html>".to_string()),
            &mut errors,
        );

        assert_eq!(ip, None);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("invalid ip response"));
    }

    /// 驗證 `get_public_ip` 的單一來源處理邏輯會保留原始服務錯誤。
    #[test]
    fn test_get_public_ip_collects_upstream_error() {
        let mut errors = Vec::new();

        let ip = collect_public_ip_result("seeip", Err(anyhow!("timeout")), &mut errors);
        assert_eq!(ip, None);
        assert_eq!(errors, vec!["seeip: timeout".to_string()]);
    }

    /// Live 測試：直接呼叫 `get_public_ip()` 取得目前對外 IP。
    ///
    /// 此測試會對外發 HTTP 請求，因此預設標記為 `ignored`，
    /// 需要時再手動執行。
    #[tokio::test]
    #[ignore]
    async fn test_get_public_ip() {
        let ip = get_public_ip().await.unwrap();

        println!("public_ip={ip}");
        assert!(ip.parse::<IpAddr>().is_ok());
    }
}
