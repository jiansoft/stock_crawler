use std::{
    net::IpAddr,
    sync::atomic::{AtomicUsize, Ordering},
};

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use scraper::{ElementRef, Html, Selector};

use crate::core::declare::{StockExchange, StockExchangeMarket};
use crate::infra::crawler::{CrawlerError, bigdatacloud, myip};
use crate::{
    core::util::{self, map::Keyable, text},
    infra::crawler::{ipconfig, ipify, ipinfo, seeip},
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
    /// 交易市場。
    pub market: StockExchangeMarket,
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
) -> Result<Vec<AnnualProfit>, CrawlerError> {
    let text = util::http::get(url, None)
        .await
        .map_err(|e| CrawlerError::Network(e.to_string()))?;
    let document = Html::parse_document(&text);
    let selector = Selector::parse("#oMainTable > tbody > tr:nth-child(n+4)")
        .map_err(|why| CrawlerError::Scraper(format!("{why:?}")))?;
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
pub async fn get_public_ip() -> Result<String, CrawlerError> {
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

    Err(CrawlerError::EmptyResponse(format!(
        "Failed to get public IP from all services: {}",
        errors.join(" | ")
    )))
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
fn normalize_public_ip(service_name: &str, ip: &str) -> Result<String, CrawlerError> {
    let normalized = ip.trim();

    if normalized.is_empty() {
        return Err(CrawlerError::EmptyResponse(format!(
            "{service_name}: empty response"
        )));
    }

    normalized
        .parse::<IpAddr>()
        .map(|ip| ip.to_string())
        .map_err(|why| {
            CrawlerError::Parse(format!(
                "{service_name}: invalid ip response `{normalized}` because {why}"
            ))
        })
}

/// 外資及陸資持股狀況爬蟲載體 (DTO)。
///
/// 用於存取從 TWSE 或 TPEx 採集到的外資及陸資持股統計基本資料。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QfiiDto {
    /// 證券代號
    pub stock_symbol: String,
    /// 已發行股數
    pub issued_share: i64,
    /// 全體外資及陸資持有股數
    pub shares_held: i64,
    /// 全體外資及陸資持股比率
    pub share_holding_percentage: Decimal,
}

/// 營收資訊爬蟲載體 (DTO)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevenueDto {
    /// 股票代號
    pub stock_symbol: String,
    /// 當月營收
    pub monthly: Decimal,
    /// 上月營收
    pub last_month: Decimal,
    /// 去年當月營收
    pub last_year_this_month: Decimal,
    /// 當月累計營收
    pub monthly_accumulated: Decimal,
    /// 去年累計營收
    pub last_year_monthly_accumulated: Decimal,
    /// 上月比較增減(%)
    pub compared_with_last_month: Decimal,
    /// 去年同月增減(%)
    pub compared_with_last_year_same_month: Decimal,
    /// 前期比較增減(%)
    pub accumulated_compared_with_last_year: Decimal,
    /// 營收月份 (YYYYMM 格式整數，如 202605)
    pub date: i64,
}

impl From<Vec<String>> for RevenueDto {
    fn from(item: Vec<String>) -> Self {
        use std::str::FromStr;
        let stock_symbol = item[0].to_string();

        let monthly = {
            let s = item[2].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!("Failed to parse 'monthly'({}) field: {}", item[2], err);
                    Default::default()
                })
            }
        };
        let last_month = {
            let s = item[3].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!("Failed to parse 'last_month'({}) field: {}", item[3], err);
                    Default::default()
                })
            }
        };
        let last_year_this_month = {
            let s = item[4].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!(
                        "Failed to parse 'last_year_this_month'({}) field: {}",
                        item[4], err
                    );
                    Default::default()
                })
            }
        };
        let monthly_accumulated = {
            let s = item[7].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!(
                        "Failed to parse 'monthly_accumulated'({}) field: {}",
                        item[7], err
                    );
                    Default::default()
                })
            }
        };
        let last_year_monthly_accumulated = {
            let s = item[8].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!(
                        "Failed to parse 'last_year_monthly_accumulated'({}) field: {}",
                        item[8], err
                    );
                    Default::default()
                })
            }
        };
        let compared_with_last_month = {
            let s = item[5].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!(
                        "Failed to parse 'compared_with_last_month'({}) field: {}",
                        item[5], err
                    );
                    Default::default()
                })
            }
        };
        let compared_with_last_year_same_month = {
            let s = item[6].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!(
                        "Failed to parse 'compared_with_last_year_same_month'({}) field: {}",
                        item[6], err
                    );
                    Default::default()
                })
            }
        };
        let accumulated_compared_with_last_year = {
            let s = item[9].replace([',', ' '], "");
            if s.is_empty() {
                Default::default()
            } else {
                Decimal::from_str(&s).unwrap_or_else(|err| {
                    eprintln!(
                        "Failed to parse 'accumulated_compared_with_last_year'({}) field: {}",
                        item[9], err
                    );
                    Default::default()
                })
            }
        };

        Self {
            stock_symbol,
            monthly,
            last_month,
            last_year_this_month,
            monthly_accumulated,
            last_year_monthly_accumulated,
            compared_with_last_month,
            compared_with_last_year_same_month,
            accumulated_compared_with_last_year,
            date: 0,
        }
    }
}

/// 每日收盤報價欄位解析錯誤。
///
/// 這個錯誤型別讓「來源格式壞掉」與「真的沒有資料」可以被區分開來：
///
/// - [`QuoteParseError::MissingField`]：來源少了必要欄位，通常代表對方改了
///   欄位名稱或順序——舊版程式會默默補 0，導致半套資料寫進資料庫。
/// - [`QuoteParseError::InvalidDecimal`]：欄位內容不是數字、也不是已知的
///   「無資料」佔位符（例如 `--`），代表內容被污染或格式變更。
///
/// 兩種情況都應該讓該列資料被「拒絕」而不是以零值入庫；
/// 呼叫端再依拒絕比例決定要跳過少數壞列，還是整批失敗。
#[derive(Debug, thiserror::Error)]
pub enum QuoteParseError {
    /// 來源資料缺少必要欄位（欄位名稱不存在，或該列長度不足）。
    #[error("missing field `{field}` in quote row")]
    MissingField {
        /// 缺少的欄位名稱。
        field: &'static str,
    },
    /// 欄位內容無法解析成數值，且不是已知的「無資料」佔位符。
    #[error("invalid decimal for field `{field}`: `{raw}`")]
    InvalidDecimal {
        /// 解析失敗的欄位名稱。
        field: &'static str,
        /// 原始字串內容，保留下來方便對照來源網頁除錯。
        raw: String,
        /// 底層的 decimal 解析錯誤（保留 source chain）。
        #[source]
        source: rust_decimal::Error,
    },
}

/// 判斷欄位內容是否為來源的「無資料」佔位符。
///
/// TWSE/TPEx 對「當日無成交、無委買賣」的價格欄位會回傳 `--`（或多個連字號）、
/// 空字串，部分來源用 `N/A`。這些是合法的「沒有值」，應轉成 0 而不是解析錯誤；
/// 注意負數如 `-5.00` 含有數字，不會被此規則誤判。
fn is_no_data_placeholder(raw: &str) -> bool {
    let trimmed = raw.trim();
    trimmed.is_empty() || trimmed.chars().all(|c| c == '-') || trimmed.eq_ignore_ascii_case("n/a")
}

/// 解析單一報價欄位為 `Decimal`。
///
/// 規則（依序）：
/// 1. 「無資料」佔位符（`--`、空白、`N/A`）→ 回傳 0，這是合法情況。
/// 2. 移除千分位逗號後可解析 → 回傳數值。
/// 3. 其餘 → 回傳 [`QuoteParseError::InvalidDecimal`]，讓呼叫端拒絕該列，
///    而不是像舊版 `unwrap_or_default()` 那樣默默寫入 0。
pub(crate) fn parse_quote_decimal(
    field: &'static str,
    raw: &str,
) -> Result<Decimal, QuoteParseError> {
    if is_no_data_placeholder(raw) {
        return Ok(Decimal::ZERO);
    }

    // 千分位逗號（例如 "1,234,567"）不是數字的一部分，先移除再解析。
    let cleaned = raw.replace(',', "");
    cleaned
        .trim()
        .parse::<Decimal>()
        .map_err(|source| QuoteParseError::InvalidDecimal {
            field,
            raw: raw.to_owned(),
            source,
        })
}

/// 解析「軟性」報價欄位：解析失敗時記 warning 並回傳 0，而不是拒絕整列。
///
/// 用於「漲跌價差」這類欄位——除權息日等特殊情況下，來源可能在此欄放
/// 非數字標記。漲跌為 0 的傷害有限（漲跌幅稍後會用前一交易日收盤價重算），
/// 為了這一欄拒絕整列反而會遺失開高低收等重要資料，因此採取寬鬆策略。
/// 價格與量能欄位仍使用嚴格的 [`parse_quote_decimal`]。
fn parse_soft_quote_decimal(field: &'static str, raw: &str) -> Decimal {
    match parse_quote_decimal(field, raw) {
        Ok(value) => value,
        Err(why) => {
            tracing::warn!("quote field fallback to zero: {why}");
            Decimal::ZERO
        }
    }
}

/// 檢查「被拒絕的資料列」比例是否仍在容忍範圍內。
///
/// 單一壞列可能只是來源偶發雜訊，跳過即可；但大量壞列幾乎可以肯定是
/// 來源格式變更（欄位改名、欄位順序調整）。此時寧可讓整批抓取失敗、
/// 觸發告警請人來調查，也不要把大量缺漏的行情資料寫進資料庫。
///
/// 門檻：拒絕比例超過 10% 即回傳錯誤；低於門檻時只記 warning。
pub(crate) fn ensure_rejected_rows_within_threshold(
    source: &str,
    rejected: usize,
    total: usize,
) -> Result<(), CrawlerError> {
    if rejected == 0 {
        return Ok(());
    }

    // rejected * 10 > total 等價於 rejected / total > 10%，用整數運算避免浮點誤差。
    if rejected * 10 > total {
        return Err(CrawlerError::Parse(format!(
            "{source}: rejected {rejected}/{total} quote rows, source format may have changed"
        )));
    }

    tracing::warn!("{source}: rejected {rejected}/{total} quote rows during parsing");
    Ok(())
}

/// 每日收盤報價爬蟲載體 (DTO)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyQuoteDto {
    /// 股票代號
    pub symbol: String,
    /// 交易日期
    pub date: NaiveDate,
    /// 開盤價
    pub opening_price: Decimal,
    /// 最高價
    pub highest_price: Decimal,
    /// 最低價
    pub lowest_price: Decimal,
    /// 收盤價
    pub closing_price: Decimal,
    /// 漲跌價差
    pub change: Decimal,
    /// 漲跌幅（百分比）
    pub change_range: Decimal,
    /// 成交股數
    pub trading_volume: Decimal,
    /// 成交金額
    pub trade_value: Decimal,
    /// 成交筆數
    pub transaction: Decimal,
    /// 本益比
    pub price_earning_ratio: Decimal,
    /// 股價淨值比
    pub price_to_book_ratio: Decimal,
    /// 最後揭示買價
    pub last_best_bid_price: Decimal,
    /// 最後揭示買量
    pub last_best_bid_volume: Decimal,
    /// 最後揭示賣價
    pub last_best_ask_price: Decimal,
    /// 最後揭示賣量
    pub last_best_ask_volume: Decimal,
}

impl DailyQuoteDto {
    /// 建立 `DailyQuoteDto` 預設實例，並同步初始化代碼與日期。
    pub fn new<S: Into<String>>(symbol: S, date: NaiveDate) -> Self {
        Self {
            symbol: symbol.into(),
            date,
            opening_price: Decimal::ZERO,
            highest_price: Decimal::ZERO,
            lowest_price: Decimal::ZERO,
            closing_price: Decimal::ZERO,
            change: Decimal::ZERO,
            change_range: Decimal::ZERO,
            trading_volume: Decimal::ZERO,
            trade_value: Decimal::ZERO,
            transaction: Decimal::ZERO,
            price_earning_ratio: Decimal::ZERO,
            price_to_book_ratio: Decimal::ZERO,
            last_best_bid_price: Decimal::ZERO,
            last_best_bid_volume: Decimal::ZERO,
            last_best_ask_price: Decimal::ZERO,
            last_best_ask_volume: Decimal::ZERO,
        }
    }

    /// 依欄位名稱映射，從單筆原始字串資料建立 `DailyQuoteDto`。
    ///
    /// `map` 是「欄位名稱 → 欄位索引」的對照表（由呼叫端從來源的表頭建立），
    /// 適用於欄位順序可能變動的來源（例如 TWSE MI_INDEX）。
    ///
    /// # Errors
    ///
    /// - 任何必要欄位在 `map` 中不存在、或該列長度不足時，
    ///   回傳 [`QuoteParseError::MissingField`]（通常代表來源改了欄位名稱）。
    /// - 欄位內容不是數字、也不是 `--` 等「無資料」佔位符時，
    ///   回傳 [`QuoteParseError::InvalidDecimal`]。
    ///
    /// 舊版對上述兩種情況都默默補 0，導致資料庫可能寫入「部分欄位為零」的
    /// 半套行情；現在改為把整列拒絕，由呼叫端統計拒絕比例決定後續處理。
    pub fn from_with_map(
        item: &[String],
        map: &std::collections::HashMap<&str, usize>,
        date: NaiveDate,
    ) -> Result<Self, QuoteParseError> {
        // 依欄位名稱取出原始字串；欄位不存在或索引超界都視為結構性錯誤。
        let get_field = |field: &'static str| -> Result<&String, QuoteParseError> {
            map.get(field)
                .and_then(|&i| item.get(i))
                .ok_or(QuoteParseError::MissingField { field })
        };

        let code = get_field("證券代號")?.clone();
        let mut dto = DailyQuoteDto::new(code, date);

        // 逐一解析數值欄位；任何一欄失敗都會讓整列被拒絕（? 提早返回）。
        dto.trading_volume = parse_quote_decimal("成交股數", get_field("成交股數")?)?;
        dto.transaction = parse_quote_decimal("成交筆數", get_field("成交筆數")?)?;
        dto.trade_value = parse_quote_decimal("成交金額", get_field("成交金額")?)?;
        dto.opening_price = parse_quote_decimal("開盤價", get_field("開盤價")?)?;
        dto.highest_price = parse_quote_decimal("最高價", get_field("最高價")?)?;
        dto.lowest_price = parse_quote_decimal("最低價", get_field("最低價")?)?;
        dto.closing_price = parse_quote_decimal("收盤價", get_field("收盤價")?)?;
        // 漲跌價差採「軟性」解析：除權息日此欄可能是非數字標記，補 0 不拒絕整列。
        dto.change = parse_soft_quote_decimal("漲跌價差", get_field("漲跌價差")?);
        dto.last_best_bid_price = parse_quote_decimal("最後揭示買價", get_field("最後揭示買價")?)?;
        dto.last_best_bid_volume = parse_quote_decimal("最後揭示買量", get_field("最後揭示買量")?)?;
        dto.last_best_ask_price = parse_quote_decimal("最後揭示賣價", get_field("最後揭示賣價")?)?;
        dto.last_best_ask_volume = parse_quote_decimal("最後揭示賣量", get_field("最後揭示賣量")?)?;
        dto.price_earning_ratio = parse_quote_decimal("本益比", get_field("本益比")?)?;

        // 處理漲跌符號。TWSE 把「漲/跌」方向放在獨立欄位（+/- 或紅/綠字），
        // 漲跌價差本身是無號數。這個欄位若被改名而遺失，漲跌方向會整批出錯，
        // 因此也視為必要欄位。
        let sign = get_field("漲跌(+/-)")?;
        if sign.contains('-') || sign.contains('綠') {
            dto.change = -dto.change.abs();
        } else if sign.contains('+') || sign.contains('紅') {
            dto.change = dto.change.abs();
        }

        Ok(dto)
    }

    /// 在給定交易所與日期的前提下，將「固定欄位順序」的來源資料轉成 `DailyQuoteDto`。
    ///
    /// 與 [`Self::from_with_map`] 的差別：這裡的來源（例如 TPEx 收盤行情）以
    /// 位置（索引）而不是欄位名稱對應欄位。
    ///
    /// # Errors
    ///
    /// 該列長度不足（缺欄位）回傳 [`QuoteParseError::MissingField`]；
    /// 欄位內容無法解析且非「無資料」佔位符時回傳 [`QuoteParseError::InvalidDecimal`]。
    pub fn from_with_exchange(
        exchange: StockExchange,
        item: &[String],
        date: NaiveDate,
    ) -> Result<Self, QuoteParseError> {
        // 第 0 欄固定是證券代號；整列為空時直接視為缺欄位。
        let symbol = item.first().ok_or(QuoteParseError::MissingField {
            field: "證券代號"
        })?;
        let mut dto = DailyQuoteDto::new(symbol.to_string(), date);

        match exchange {
            StockExchange::TWSE => {
                // (索引, 欄位名稱)。欄位名稱只用於錯誤訊息，讓除錯時能對照來源表頭。
                let decimal_fields = [
                    (2, "成交股數", &mut dto.trading_volume),
                    (3, "成交筆數", &mut dto.transaction),
                    (4, "成交金額", &mut dto.trade_value),
                    (5, "開盤價", &mut dto.opening_price),
                    (6, "最高價", &mut dto.highest_price),
                    (7, "最低價", &mut dto.lowest_price),
                    (8, "收盤價", &mut dto.closing_price),
                    (11, "最後揭示買價", &mut dto.last_best_bid_price),
                    (12, "最後揭示買量", &mut dto.last_best_bid_volume),
                    (13, "最後揭示賣價", &mut dto.last_best_ask_price),
                    (14, "最後揭示賣量", &mut dto.last_best_ask_volume),
                    (15, "本益比", &mut dto.price_earning_ratio),
                ];

                for (index, field, target) in decimal_fields {
                    let raw = item
                        .get(index)
                        .ok_or(QuoteParseError::MissingField { field })?;
                    *target = parse_quote_decimal(field, raw)?;
                }

                // 第 10 欄是漲跌價差，採「軟性」解析：除權息日此欄可能是
                // 非數字標記，補 0 不拒絕整列（缺欄位仍是硬錯誤）。
                let change_raw = item.get(10).ok_or(QuoteParseError::MissingField {
                    field: "漲跌價差",
                })?;
                dto.change = parse_soft_quote_decimal("漲跌價差", change_raw);

                // 第 9 欄是漲跌方向（HTML 內含 + 或 -）；含 '-' 時把漲跌價差轉負。
                let sign = item.get(9).ok_or(QuoteParseError::MissingField {
                    field: "漲跌(+/-)",
                })?;
                if sign.contains('-') {
                    dto.change = -dto.change;
                }
            }
            StockExchange::TPEx => {
                // TPEx 的漲跌欄（索引 3）自帶正負號，不需要獨立的方向欄位。
                let decimal_fields = [
                    (7, "成交股數", &mut dto.trading_volume),
                    (9, "成交筆數", &mut dto.transaction),
                    (8, "成交金額", &mut dto.trade_value),
                    (4, "開盤價", &mut dto.opening_price),
                    (5, "最高價", &mut dto.highest_price),
                    (6, "最低價", &mut dto.lowest_price),
                    (2, "收盤價", &mut dto.closing_price),
                    (10, "最後揭示買價", &mut dto.last_best_bid_price),
                    (11, "最後揭示買量", &mut dto.last_best_bid_volume),
                    (12, "最後揭示賣價", &mut dto.last_best_ask_price),
                    (13, "最後揭示賣量", &mut dto.last_best_ask_volume),
                ];

                for (index, field, target) in decimal_fields {
                    let raw = item
                        .get(index)
                        .ok_or(QuoteParseError::MissingField { field })?;
                    *target = parse_quote_decimal(field, raw)?;
                }

                // 第 3 欄是自帶正負號的漲跌，採「軟性」解析：
                // 除權息日此欄可能是非數字標記，補 0 不拒絕整列。
                let change_raw = item
                    .get(3)
                    .ok_or(QuoteParseError::MissingField { field: "漲跌" })?;
                dto.change = parse_soft_quote_decimal("漲跌", change_raw);
            }
            _ => {}
        }

        Ok(dto)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::IpAddr;

    use anyhow::anyhow;
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::core::declare::StockExchange;

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

    // === 報價欄位解析（typed error）測試 ===

    /// 驗證「無資料」佔位符（`--`、空白、`N/A`）規則化為 0，屬合法情況。
    #[test]
    fn parse_quote_decimal_treats_placeholders_as_zero() {
        assert_eq!(parse_quote_decimal("開盤價", "--").unwrap(), Decimal::ZERO);
        assert_eq!(parse_quote_decimal("開盤價", "").unwrap(), Decimal::ZERO);
        assert_eq!(parse_quote_decimal("開盤價", "  ").unwrap(), Decimal::ZERO);
        assert_eq!(
            parse_quote_decimal("開盤價", "----").unwrap(),
            Decimal::ZERO
        );
        assert_eq!(parse_quote_decimal("本益比", "N/A").unwrap(), Decimal::ZERO);
    }

    /// 驗證千分位逗號與正負號都能正確解析，負數不會被誤判為佔位符。
    #[test]
    fn parse_quote_decimal_parses_commas_and_signs() {
        assert_eq!(
            parse_quote_decimal("成交股數", "1,234,567").unwrap(),
            dec!(1234567)
        );
        assert_eq!(parse_quote_decimal("漲跌", "-5.00").unwrap(), dec!(-5.00));
        assert_eq!(parse_quote_decimal("漲跌", "+3.5").unwrap(), dec!(3.5));
    }

    /// 驗證垃圾內容回傳 InvalidDecimal，而不是像舊版一樣默默補 0。
    #[test]
    fn parse_quote_decimal_rejects_garbage() {
        let err = parse_quote_decimal("收盤價", "abc").unwrap_err();

        match err {
            QuoteParseError::InvalidDecimal { field, raw, .. } => {
                assert_eq!(field, "收盤價");
                assert_eq!(raw, "abc");
            }
            other => panic!("expected InvalidDecimal, got {other:?}"),
        }
    }

    /// 驗證拒絕比例門檻：0 筆或低於 10% 通過，超過 10% 整批失敗。
    #[test]
    fn rejected_rows_threshold_allows_minor_and_blocks_major() {
        assert!(ensure_rejected_rows_within_threshold("TWSE", 0, 100).is_ok());
        assert!(ensure_rejected_rows_within_threshold("TWSE", 5, 100).is_ok());
        assert!(ensure_rejected_rows_within_threshold("TWSE", 11, 100).is_err());
        // 小樣本也適用：5 列壞 1 列（20%）應整批失敗。
        assert!(ensure_rejected_rows_within_threshold("TPEx", 1, 5).is_err());
    }

    /// 建立 TWSE 欄位名稱對照表與一筆有效資料列，供 from_with_map 測試共用。
    fn twse_map_and_row() -> (HashMap<&'static str, usize>, Vec<String>) {
        let map = HashMap::from([
            ("證券代號", 0),
            ("成交股數", 1),
            ("成交筆數", 2),
            ("成交金額", 3),
            ("開盤價", 4),
            ("最高價", 5),
            ("最低價", 6),
            ("收盤價", 7),
            ("漲跌(+/-)", 8),
            ("漲跌價差", 9),
            ("最後揭示買價", 10),
            ("最後揭示買量", 11),
            ("最後揭示賣價", 12),
            ("最後揭示賣量", 13),
            ("本益比", 14),
        ]);
        let row = vec![
            "2330".to_string(),
            "1,234,000".to_string(),
            "5,678".to_string(),
            "987,654,321".to_string(),
            "950.5".to_string(),
            "960.5".to_string(),
            "945.5".to_string(),
            "955.5".to_string(),
            "綠".to_string(),
            "12.5".to_string(),
            "955.0".to_string(),
            "100".to_string(),
            "956.0".to_string(),
            "200".to_string(),
            "20.5".to_string(),
        ];
        (map, row)
    }

    /// 驗證 from_with_map 正常解析（含千分位、綠字轉負值）。
    #[test]
    fn from_with_map_parses_valid_row() {
        let (map, row) = twse_map_and_row();
        let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

        let dto = DailyQuoteDto::from_with_map(&row, &map, date).unwrap();

        assert_eq!(dto.symbol, "2330");
        assert_eq!(dto.trading_volume, dec!(1234000));
        assert_eq!(dto.closing_price, dec!(955.5));
        // 「綠」代表下跌，漲跌價差應轉為負值。
        assert_eq!(dto.change, dec!(-12.5));
        assert_eq!(dto.price_earning_ratio, dec!(20.5));
    }

    /// 驗證欄位改名（對照表缺欄位）時整列被拒絕，而不是補 0。
    #[test]
    fn from_with_map_rejects_missing_column() {
        let (mut map, row) = twse_map_and_row();
        // 模擬來源把「收盤價」改名：對照表中不再有這個欄位。
        map.remove("收盤價");
        let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

        let err = DailyQuoteDto::from_with_map(&row, &map, date).unwrap_err();

        assert!(matches!(
            err,
            QuoteParseError::MissingField { field: "收盤價" }
        ));
    }

    /// 驗證價格欄位含垃圾內容時整列被拒絕。
    #[test]
    fn from_with_map_rejects_garbage_price() {
        let (map, mut row) = twse_map_and_row();
        row[7] = "corrupted".to_string(); // 收盤價
        let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

        let err = DailyQuoteDto::from_with_map(&row, &map, date).unwrap_err();

        assert!(matches!(err, QuoteParseError::InvalidDecimal { .. }));
    }

    /// 驗證漲跌價差是「軟性」欄位：非數字標記補 0，不拒絕整列。
    #[test]
    fn from_with_map_soft_change_falls_back_to_zero() {
        let (map, mut row) = twse_map_and_row();
        row[9] = "除息".to_string(); // 漲跌價差
        let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

        let dto = DailyQuoteDto::from_with_map(&row, &map, date).unwrap();

        assert_eq!(dto.change, Decimal::ZERO);
        // 其餘欄位仍完整保留。
        assert_eq!(dto.closing_price, dec!(955.5));
    }

    /// 驗證 TPEx 位置式解析：正常列成功、含自帶正負號的漲跌。
    #[test]
    fn from_with_exchange_tpex_parses_valid_row() {
        let row = vec![
            "5483".to_string(),      // 0: 代號
            "中美晶".to_string(),    // 1: 名稱
            "100.00".to_string(),    // 2: 收盤價
            "-1.50".to_string(),     // 3: 漲跌（自帶負號）
            "98.50".to_string(),     // 4: 開盤價
            "101.00".to_string(),    // 5: 最高價
            "98.00".to_string(),     // 6: 最低價
            "10,000".to_string(),    // 7: 成交股數
            "1,000,000".to_string(), // 8: 成交金額
            "500".to_string(),       // 9: 成交筆數
            "100.00".to_string(),    // 10: 最後買價
            "10".to_string(),        // 11: 最後買量
            "100.50".to_string(),    // 12: 最後賣價
            "20".to_string(),        // 13: 最後賣量
        ];
        let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

        let dto = DailyQuoteDto::from_with_exchange(StockExchange::TPEx, &row, date).unwrap();

        assert_eq!(dto.symbol, "5483");
        assert_eq!(dto.closing_price, dec!(100.00));
        assert_eq!(dto.change, dec!(-1.50));
        assert_eq!(dto.trading_volume, dec!(10000));
    }

    /// 驗證 TPEx 資料列長度不足（缺欄位）時整列被拒絕。
    #[test]
    fn from_with_exchange_tpex_rejects_short_row() {
        let row = vec!["5483".to_string(), "中美晶".to_string()];
        let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

        let err = DailyQuoteDto::from_with_exchange(StockExchange::TPEx, &row, date).unwrap_err();

        assert!(matches!(err, QuoteParseError::MissingField { .. }));
    }
}
