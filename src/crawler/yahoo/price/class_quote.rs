//! # Yahoo 類股行情 JSON API
//!
//! 此模組負責與 Yahoo 類股頁背後使用的 JSON API 溝通。
//! 主要工作包含：
//! - 列舉所有需要輪詢的類股分類。
//! - 組裝 `StockServices.getClassQuotes` 的請求 URL。
//! - 處理 `offset` 分頁，直到抓完整個類股。
//! - 將 Yahoo 的原始 JSON 欄位轉成 [`RealtimeSnapshot`]。

use std::{collections::HashMap, str::FromStr, time::Duration};

use anyhow::{anyhow, Context, Result};
use once_cell::sync::OnceCell;
use reqwest::Client;
use rust_decimal::Decimal;
use serde::Deserialize;
use tokio::time::sleep;

use crate::{
    cache::RealtimeSnapshot,
    crawler::yahoo::{self, YahooClassCategory, YahooClassExchange},
    logging, util,
};

/// Yahoo 類股行情 JSON API 的基底 URL。
const CLASS_QUOTES_API_URL: &str =
    "https://tw.stock.yahoo.com/_td-stock/api/resource/StockServices.getClassQuotes";
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

/// 單一分頁抓取後的最小結果。
///
/// 這裡只保留背景輪詢下一步真正需要的資料，
/// 避免把完整 JSON 結構與整頁字串生命週期拉長到頁外。
struct ClassQuotesPageResult {
    snapshots: Vec<(String, RealtimeSnapshot)>,
    raw_item_count: usize,
    results_total: usize,
    next_offset: Option<usize>,
}

/// Yahoo `getClassQuotes` API 的單頁回應。
///
/// 只保留 crawler 真正需要的欄位：
/// - `list`：單頁的股票列表。
/// - `pagination`：分頁資訊。
#[derive(Debug, Default, Deserialize)]
struct ClassQuotesResponse<'a> {
    #[serde(default)]
    #[serde(borrow)]
    list: Vec<ClassQuoteItem<'a>>,
    #[serde(default)]
    pagination: ClassQuotesPagination<'a>,
}

/// Yahoo 單筆類股項目。
///
/// 欄位型別盡量使用 borrowed data，降低盤中長輪詢時的暫時配置量。
#[derive(Debug, Deserialize)]
struct ClassQuoteItem<'a> {
    #[serde(default, borrow)]
    symbol: Option<&'a str>,
    #[serde(default, borrow, rename = "symbolName")]
    symbol_name: Option<&'a str>,
    #[serde(default, borrow, rename = "systexId")]
    systex_id: Option<&'a str>,
    #[serde(default)]
    price: Option<RawNumericField<'a>>,
    #[serde(default)]
    change: Option<RawNumericField<'a>>,
    #[serde(default, borrow, rename = "changePercent")]
    change_percent: Option<RawNumericValue<'a>>,
    #[serde(default, rename = "regularMarketOpen")]
    regular_market_open: Option<RawNumericField<'a>>,
    #[serde(default, rename = "regularMarketDayHigh")]
    regular_market_day_high: Option<RawNumericField<'a>>,
    #[serde(default, rename = "regularMarketDayLow")]
    regular_market_day_low: Option<RawNumericField<'a>>,
    #[serde(default, rename = "regularMarketPreviousClose")]
    regular_market_previous_close: Option<RawNumericField<'a>>,
    #[serde(default, borrow, rename = "volumeK")]
    volume_k: Option<RawNumericValue<'a>>,
}

/// Yahoo 把不少數值欄位包成 `{ raw: ... }` 物件。
#[derive(Debug, Default, Deserialize)]
struct RawNumericField<'a> {
    #[serde(default, borrow)]
    raw: Option<RawNumericValue<'a>>,
}

/// Yahoo 數值欄位可能是字串、數字或 `null`。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawNumericValue<'a> {
    Text(&'a str),
    Number(serde_json::Number),
}

/// Yahoo 類股 API 的分頁資訊。
#[derive(Debug, Default, Deserialize)]
struct ClassQuotesPagination<'a> {
    #[serde(default, rename = "resultsTotal")]
    results_total: usize,
    #[serde(default, borrow, rename = "nextOffset")]
    next_offset: Option<&'a str>,
}

impl ClassQuotesPagination<'_> {
    /// 將 Yahoo 原始字串格式的 `nextOffset` 轉成數值。
    fn next_offset(&self) -> Result<Option<usize>> {
        self.next_offset
            .as_deref()
            .map(|offset| {
                offset.parse::<usize>().with_context(|| {
                    format!("Failed to parse Yahoo class quote nextOffset: {offset}")
                })
            })
            .transpose()
    }
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
            .zstd(true)
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(15))
            .tcp_nodelay(true)
            .tcp_keepalive(Duration::from_secs(30))
            .http1_only()
            .pool_max_idle_per_host(1)
            .pool_idle_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::limited(5))
            .referer(true)
            .user_agent(util::http::user_agent::gen_random_ua())
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
    // 用 symbol -> snapshot 收集完整類股結果，
    // 後面若同頁或跨頁出現重複代號，後寫入的值會覆蓋前值。
    let mut snapshots = HashMap::new();
    // `offset` 是 Yahoo 分頁的游標；首頁永遠從 0 開始。
    let mut offset = 0usize;
    // `page` 只用來判斷「首頁是否為空」這種特殊錯誤情境。
    let mut page = 0usize;
    let mut page_count = 0usize;
    let mut raw_item_count = 0usize;

    loop {
        // 每一圈只抓一頁，抓完再看 `nextOffset` 決定要不要繼續。
        let page_result = fetch_class_quotes_page(category, offset).await?;
        let list_len = page_result.raw_item_count;
        page_count += 1;
        raw_item_count += list_len;

        // 首頁就空通常代表 URL 參數、來源行為或類股設定有問題，
        // 這種情況不應該靜默吞掉，直接回錯讓呼叫端知道這個類股整輪失敗。
        if list_len == 0 && page == 0 {
            let error_message = format!(
                "Yahoo 類股 API 首頁為空: {} {}({})",
                category.exchange.label(),
                category.name,
                category.sector_id
            );
            logging::error_file_async(error_message.clone());
            return Err(anyhow!(error_message));
        }

        // 首頁的 `resultsTotal` 最接近這個類股的最終容量，
        // 提前保留空間可以減少整輪插入時的 rehash 與額外配置。
        if page == 0 && page_result.results_total > 0 {
            snapshots.reserve(page_result.results_total);
        }

        // 單頁內每一筆 Yahoo item 都轉成專案內部的 `RealtimeSnapshot`。
        // 無法辨識股票代號的資料會被 `parse_class_quote_item` 主動略過。
        for (symbol, snapshot) in page_result.snapshots {
            snapshots.insert(symbol, snapshot);
        }

        // 只有 `nextOffset` 真正往前推進時才繼續抓下一頁，
        // 這可以避免來源異常回傳相同 offset 造成無窮迴圈。
        match page_result.next_offset {
            Some(next_offset) if next_offset > offset => {
                // 同一個類股若有大量分頁，也要在頁與頁之間停一下，
                // 避免單一 sector 連續請求過密而被 Yahoo 視為異常流量。
                sleep(PAGE_REQUEST_INTERVAL).await;
                offset = next_offset;
                page += 1;
            }
            _ => break,
        }
    }

    // 如果整個類股抓完卻一檔都解析不出來，代表不是單頁偶發缺值，
    // 而是整個類股資料格式可能有變，這時候直接回錯比較安全。
    if snapshots.is_empty() {
        let error_message = format!(
            "Yahoo 類股 API 未解析出任何股票: {} {}({})",
            category.exchange.label(),
            category.name,
            category.sector_id
        );
        logging::error_file_async(error_message.clone());
        return Err(anyhow!(error_message));
    }

    Ok(CategoryFetchResult {
        diagnostics: CategoryFetchDiagnostics {
            page_count,
            raw_item_count,
            snapshot_count: snapshots.len(),
        },
        snapshots,
    })
}

/// 抓取單一類股分頁的原始 JSON 回應。
///
/// # Errors
/// - HTTP 失敗。
/// - JSON 解析失敗。
/// - 非首頁卻出現「有總筆數但空列表」的異常資料。
async fn fetch_class_quotes_page(
    category: &YahooClassCategory,
    offset: usize,
) -> Result<ClassQuotesPageResult> {
    // 先把 URL 組在一起，讓日誌與測試都能重用同一套規則。
    let url = build_class_quotes_api_url(category, offset);
    // 這裡直接拿 response bytes 後立刻解析，
    // 讓 borrowed JSON 欄位可以只活在單頁處理期間，降低暫時配置壓力。
    let response =
        util::http::get_response_with_client(get_yahoo_class_quotes_client()?, &url, None).await?;
    let status = response.status();
    let response_body = response.bytes().await.map_err(|e| {
        anyhow!(
            "Error reading Yahoo class quote response body from {}: {}",
            url,
            e
        )
    })?;
    let response_body_preview = String::from_utf8_lossy(response_body.as_ref());

    if !status.is_success() {
        return Err(anyhow!(
            "Yahoo 類股 API request failed with status {} for {}. Body: {}",
            status,
            url,
            util::text::truncate(&response_body_preview, 200)
        ));
    }

    let response: ClassQuotesResponse<'_> = serde_json::from_slice(response_body.as_ref())
        .map_err(|e| {
            anyhow!(
                "Failed to parse Yahoo 類股 API response from {}: {:?}. Body: {}",
                url,
                e,
                util::text::truncate(&response_body_preview, 200)
            )
        })?;
    let raw_item_count = response.list.len();
    let results_total = response.pagination.results_total;
    let next_offset = response.pagination.next_offset()?;

    // 就算首頁沒有直接回錯，也先把「整頁空資料」寫到日誌，
    // 方便後續人工從 log 追查是 Yahoo schema 變更、類股失效還是被擋流量。
    if raw_item_count == 0 {
        logging::error_file_async(format!(
            "Yahoo 類股 API 回空資料: {} {}({}) offset={} resultsTotal={} url={}",
            category.exchange.label(),
            category.name,
            category.sector_id,
            offset,
            results_total,
            url
        ));
    }

    // 非首頁如果宣稱有總筆數卻回空列表，通常代表中間頁資料異常，
    // 直接報錯，避免把「只抓到前半段」當成正常成功。
    if raw_item_count == 0 && results_total > 0 && offset > 0 {
        return Err(anyhow!(
            "Yahoo 類股 API offset={} 返回空列表但 resultsTotal={}",
            offset,
            results_total
        ));
    }

    let mut snapshots = Vec::with_capacity(raw_item_count);
    for item in &response.list {
        if let Some((symbol, snapshot)) = parse_class_quote_item(item)? {
            snapshots.push((symbol, snapshot));
        }
    }

    Ok(ClassQuotesPageResult {
        snapshots,
        raw_item_count,
        results_total,
        next_offset,
    })
}

/// 將單筆 Yahoo 類股 API 項目轉成內部快照型別。
///
/// 若該筆資料沒有可辨識的股票代號，會回傳 `Ok(None)` 讓呼叫端略過。
fn parse_class_quote_item(item: &ClassQuoteItem<'_>) -> Result<Option<(String, RealtimeSnapshot)>> {
    // Yahoo 有時同時給 `systexId` 與 `symbol`，有時只有其中一個。
    // 這裡優先採用較乾淨、穩定的 `systexId`；缺失時才退回 `2330.TW -> 2330` 這種裁切。
    let symbol = match item
        .systex_id
        .map(str::trim)
        .filter(|symbol| !symbol.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| item.symbol.map(strip_market_suffix))
    {
        Some(symbol) => symbol,
        None => return Ok(None),
    };

    // 先以 price 建立最小快照，再逐欄補齊其他資訊。
    // 這樣可以確保最重要的欄位一開始就存在，也讓預設值策略集中在 `RealtimeSnapshot::new`。
    let mut snapshot = RealtimeSnapshot::new(
        symbol.clone(),
        decimal_from_raw_field(item.price.as_ref(), &symbol, "price")?,
    );
    // 名稱如果缺失就退空字串，不因單一欄位缺值整筆報價失敗。
    snapshot.name = item.symbol_name.unwrap_or_default().trim().to_string();
    snapshot.source_site = "Yahoo".to_string();
    // 其餘欄位都透過 `decimal_at` 走一致的缺值與型別轉換規則，
    // 避免每個欄位各自寫一套解析分支。
    snapshot.change = decimal_from_raw_field(item.change.as_ref(), &symbol, "change")?;
    snapshot.change_range = decimal_at(item.change_percent.as_ref(), &symbol, "changePercent")?;
    snapshot.open = decimal_from_raw_field(item.regular_market_open.as_ref(), &symbol, "open")?;
    snapshot.high = decimal_from_raw_field(item.regular_market_day_high.as_ref(), &symbol, "high")?;
    snapshot.low = decimal_from_raw_field(item.regular_market_day_low.as_ref(), &symbol, "low")?;
    snapshot.last_close = decimal_from_raw_field(
        item.regular_market_previous_close.as_ref(),
        &symbol,
        "last_close",
    )?;
    snapshot.volume = decimal_at(item.volume_k.as_ref(), &symbol, "volumeK")?;

    Ok(Some((symbol, snapshot)))
}

/// 移除 Yahoo 股票代號的市場尾碼，例如 `2330.TW -> 2330`。
fn strip_market_suffix(symbol: &str) -> String {
    // Yahoo 常用 `2330.TW` / `6488.TWO` 這種代號；
    // 專案內部一律用純數字股號，所以只取 `.` 前半段。
    symbol
        .split('.')
        .next()
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

/// 解析 `{ raw: ... }` 形狀的 Yahoo 欄位。
fn decimal_from_raw_field(
    field: Option<&RawNumericField<'_>>,
    symbol: &str,
    field_name: &str,
) -> Result<Decimal> {
    decimal_at(
        field.and_then(|field| field.raw.as_ref()),
        symbol,
        field_name,
    )
}

/// 從 Yahoo 已解構出的欄位值轉成 `Decimal`。
///
/// 若欄位不存在，回傳 `Decimal::ZERO` 作為缺值。
fn decimal_at(
    value: Option<&RawNumericValue<'_>>,
    symbol: &str,
    field_name: &str,
) -> Result<Decimal> {
    match value {
        Some(value) => decimal_from_value(value, symbol, field_name),
        None => Ok(Decimal::ZERO),
    }
}

/// 將 Yahoo JSON 值轉成 `Decimal`。
///
/// 支援的型別為：
/// - `null`
/// - `string`
/// - `number`
fn decimal_from_value(
    value: &RawNumericValue<'_>,
    symbol: &str,
    field_name: &str,
) -> Result<Decimal> {
    match value {
        // 文字型數值交給專門函式處理，因為它還要兼顧 `-`、`市價`、`%` 等特殊字串。
        RawNumericValue::Text(text) => parse_decimal_text(text, symbol, field_name),
        // 數字型別則直接轉 `Decimal`，這是最乾淨的路徑。
        RawNumericValue::Number(number) => {
            Decimal::from_str(&number.to_string()).with_context(|| {
                format!(
                    "Failed to parse Yahoo {} as Decimal for {}: {}",
                    field_name, symbol, number
                )
            })
        }
    }
}

/// 解析 Yahoo 回傳的文字型數值欄位。
///
/// `-`、`--`、`市價` 與空字串會視為缺值並轉成 `0`。
fn parse_decimal_text(text: &str, symbol: &str, field_name: &str) -> Result<Decimal> {
    // 先 trim，避免前後空白造成解析失敗。
    let normalized = text.trim();
    // Yahoo 會用 `-`、`--`、`市價` 表示沒有固定數值，
    // 這些在本專案裡都統一視為 0，讓下游可以用同一種缺值判斷。
    if normalized.is_empty() || normalized == "-" || normalized == "--" || normalized == "市價" {
        return Ok(Decimal::ZERO);
    }

    // 真正的文字數字解析交給共用 text helper，
    // 並順手移掉逗號與百分號。
    crate::util::text::parse_decimal(normalized, Some(vec![',', '%'])).with_context(|| {
        format!(
            "Failed to parse Yahoo {} for {}: {}",
            field_name, symbol, text
        )
    })
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;
    use serde_json::json;

    use super::*;

    fn parse_test_item(value: serde_json::Value) -> Result<Option<(String, RealtimeSnapshot)>> {
        let raw = serde_json::to_vec(&value).unwrap();
        let item: ClassQuoteItem<'_> = serde_json::from_slice(&raw).unwrap();
        parse_class_quote_item(&item)
    }

    /// 驗證類股 API URL 會使用 Yahoo 實際可用的分號參數格式。
    #[test]
    fn test_build_class_quotes_api_url() {
        let category = YahooClassCategory::enabled(YahooClassExchange::Listed, 40, "半導體");

        assert_eq!(
            build_class_quotes_api_url(&category, 30),
            "https://tw.stock.yahoo.com/_td-stock/api/resource/StockServices.getClassQuotes;exchange=TAI;sectorId=40;offset=30"
        );
    }

    /// 驗證類股 JSON 會優先使用 `systexId` 當成股票代號，並正確解析各欄位。
    #[test]
    fn test_parse_class_quote_item_uses_systex_id_and_volume_k() {
        let item = json!({
            "symbol": "2330.TW",
            "symbolName": "台積電",
            "systexId": "2330",
            "price": { "raw": "998" },
            "change": { "raw": "-12" },
            "changePercent": "-1.19%",
            "regularMarketOpen": { "raw": "1005" },
            "regularMarketDayHigh": { "raw": "1010" },
            "regularMarketDayLow": { "raw": "995" },
            "regularMarketPreviousClose": { "raw": "1010" },
            "volumeK": 43210
        });

        let (symbol, snapshot) = parse_test_item(item).unwrap().unwrap();

        assert_eq!(symbol, "2330");
        assert_eq!(snapshot.name, "台積電");
        assert_eq!(snapshot.source_site, "Yahoo");
        assert_eq!(snapshot.price, dec!(998));
        assert_eq!(snapshot.change, dec!(-12));
        assert_eq!(snapshot.change_range, dec!(-1.19));
        assert_eq!(snapshot.open, dec!(1005));
        assert_eq!(snapshot.high, dec!(1010));
        assert_eq!(snapshot.low, dec!(995));
        assert_eq!(snapshot.last_close, dec!(1010));
        assert_eq!(snapshot.volume, dec!(43210));
    }

    /// 驗證當 `systexId` 缺失時，仍可由 `symbol` 去掉市場尾碼後取得股票代號。
    #[test]
    fn test_parse_class_quote_item_falls_back_to_symbol_without_suffix() {
        let item = json!({
            "symbol": "006208.TW",
            "symbolName": "富邦台50",
            "price": { "raw": "88.4" }
        });

        let (symbol, snapshot) = parse_test_item(item).unwrap().unwrap();

        assert_eq!(symbol, "006208");
        assert_eq!(snapshot.source_site, "Yahoo");
        assert_eq!(snapshot.price, dec!(88.4));
        assert_eq!(snapshot.volume, Decimal::ZERO);
    }

    /// 驗證 Yahoo 的 `nextOffset` 欄位能被正確轉成數值型態。
    #[test]
    fn test_next_offset_parsing() {
        let pagination = ClassQuotesPagination {
            results_total: 89,
            next_offset: Some("60"),
        };

        assert_eq!(pagination.next_offset().unwrap(), Some(60));
    }

    /// 驗證盤中輪詢清單會排除認購 / 認售 / 指數類，只保留實際需要採集的分類。
    #[test]
    fn test_all_class_categories_excludes_disabled_categories() {
        let categories = all_class_categories();

        assert_eq!(categories.len(), 98);
        assert!(!categories.iter().any(|category| {
            matches!(
                (category.exchange, category.sector_id),
                (YahooClassExchange::Listed, 31..=33)
                    | (YahooClassExchange::OverTheCounter, 33 | 165 | 166)
            )
        }));
        assert!(categories.iter().any(|category| {
            category.exchange == YahooClassExchange::Listed && category.sector_id == 40
        }));
    }

    /// Live 測試：驗證單一類股能抓到完整分頁資料，而不只首頁前 30 筆。
    #[tokio::test]
    #[ignore]
    async fn test_fetch_category_snapshots_integration() {
        let category = YahooClassCategory::enabled(YahooClassExchange::Listed, 40, "半導體");

        let snapshots = fetch_category_snapshots(&category).await.unwrap().snapshots;

        dbg!(&snapshots);

        assert!(
            snapshots.len() > 30,
            "expected more than first page of Yahoo class quotes"
        );

        let snapshot = snapshots.get("2330").expect("expected 2330 in TAI 40");
        assert_eq!(snapshot.symbol, "2330");
        assert_eq!(snapshot.name, "台積電");
        assert!(snapshot.price > Decimal::ZERO);
    }

    /// Live 測試：驗證上櫃半導體類股能抓到完整分頁資料。
    #[tokio::test]
    #[ignore]
    async fn test_fetch_otc_category_snapshots_integration() {
        let category =
            YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 153, "半導體");

        let snapshots = fetch_category_snapshots(&category).await.unwrap().snapshots;

        assert!(
            !snapshots.is_empty(),
            "expected some snapshots for Yahoo OTC class quotes"
        );
    }

    /// Live 測試：列出上市、上櫃、興櫃三個半導體類股下的所有股票資訊。
    #[tokio::test]
    #[ignore]
    async fn test_fetch_all_semiconductor_categories_integration() {
        let categories = [
            YahooClassCategory::enabled(YahooClassExchange::Listed, 40, "半導體"),
            YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 153, "半導體"),
            YahooClassCategory::enabled(YahooClassExchange::Emerging, 99311, "半導體"),
        ];

        for category in categories {
            let snapshots = fetch_category_snapshots(&category).await.unwrap().snapshots;
            let mut rows: Vec<_> = snapshots.into_iter().collect();
            rows.sort_by(|left, right| left.0.cmp(&right.0));

            println!(
                "\n=== {} 半導體 ({}) 共 {} 檔 ===",
                category.exchange.label(),
                category.sector_id,
                rows.len()
            );

            for (symbol, snapshot) in rows {
                println!(
                    "{}\t{}\tprice={}\tchange={}\trange={}%\topen={}\thigh={}\tlow={}\tlast_close={}\tvolume_k={}",
                    symbol,
                    snapshot.name,
                    snapshot.price,
                    snapshot.change,
                    snapshot.change_range,
                    snapshot.open,
                    snapshot.high,
                    snapshot.low,
                    snapshot.last_close,
                    snapshot.volume
                );
            }
        }
    }
}
