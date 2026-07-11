//! # Yahoo 個股基本面採集器
//!
//! 此模組負責從 Yahoo 財經抓取股票的財務比率與獲利能力指標。
//! 這些資料通常位於個股頁面的「基本」或「健診」分頁。
//!
//! ## 抓取的指標
//!
//! - **獲利能力**：營業毛利率、營業利益率、稅前/稅後淨利率。
//! - **投資報酬**：股東權益報酬率 (ROE)、資產報酬率 (ROA)。
//! - **每股指標**：每股盈餘 (EPS)、每股淨值。
//!
//! ## 實作細節
//!
//! - 使用 `once_cell::sync::Lazy` 靜態化 CSS 選擇器與正則表達式以優化效能。
//! - 採用顯式欄位賦值，便於在 Yahoo 網頁改版時快速調整對應的 Grid 索引。
//! - 具備防禦性驗證，若解析出的年份與 EPS 同時為 0，則視為採集異常。

use std::{error::Error as StdError, fmt};

use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use rust_decimal::Decimal;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use crate::{core::util, core::util::http::element, infra::crawler::yahoo::HOST};

/// 用於解析季度（如 Q1, Q2）的正則表達式
static REG_QUARTER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)q\d").expect("Failed to compile quarter regex"));

/// 個股基本資料區塊的主要選擇器
static BASE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse("#main-2-QuoteProfile-Proxy > div > section:nth-child(3)")
        .expect("Failed to parse base profile selector")
});

/// Yahoo profile 類型頁面在「無有效財務資料」時的暫時跳過快取秒數。
///
/// 這類頁面通常是新掛牌股票、暫時尚未補齊財務欄位的標的，短時間內重試
/// 通常不會得到不同結果，因此以一天作為保守的重試間隔。
pub const NO_VALID_DATA_CACHE_TTL_SECONDS: usize = 60 * 60 * 24;

/// 股票基本面與財務比率結構體
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Profile {
    /// 季度資訊 (例如: "Q4", "Q3")
    pub quarter: String,
    /// 股票代碼
    pub stock_symbol: String,
    /// 營業毛利率 (%)
    pub gross_profit: Decimal,
    /// 營業利益率 (%)
    pub operating_profit_margin: Decimal,
    /// 稅前淨利率 (%)
    pub pre_tax_income: Decimal,
    /// 稅後淨利率 (%)
    pub net_income: Decimal,
    /// 每股淨值 (元)
    pub net_asset_value_per_share: Decimal,
    /// 每股營收 (元)
    pub sales_per_share: Decimal,
    /// 每股稅後淨利 (EPS, 元)
    pub earnings_per_share: Decimal,
    /// 每股稅前淨利 (元)
    pub profit_before_tax: Decimal,
    /// 股東權益報酬率 (ROE, %)
    pub return_on_equity: Decimal,
    /// 資產報酬率 (ROA, %)
    pub return_on_assets: Decimal,
    /// 資料所屬年度 (西元)
    pub year: i32,
}

impl Profile {
    /// 建立一個新的 `Profile` 實例。
    pub fn new(stock_symbol: String) -> Self {
        Profile {
            stock_symbol,
            ..Default::default()
        }
    }
}

/// 表示 Yahoo profile 頁面存在，但未提供目前 parser 所需的財務欄位。
///
/// 這類錯誤通常不是 HTTP 失敗，而是頁面中缺少年份、EPS 或相關財務 grid，
/// 例如新掛牌股票尚未完整顯示基本面資料，或該頁面使用了不同模板。
#[derive(Debug)]
struct NoValidProfileDataError {
    stock_symbol: String,
    url: String,
}

impl fmt::Display for NoValidProfileDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Yahoo profile for {} at {} does not expose the year/EPS fields expected by the parser",
            self.stock_symbol, self.url
        )
    }
}

impl StdError for NoValidProfileDataError {}

/// 判斷錯誤是否屬於「Yahoo profile 頁面存在，但沒有可解析財務資料」。
///
/// 可用於 backfill 流程中，將這類情況視為暫時無法補抓並降低日誌等級，
/// 避免每次重試都輸出整串 backtrace。
pub fn is_no_valid_data_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<NoValidProfileDataError>().is_some()
}

/// 回傳 Yahoo profile 無有效資料的短期跳過快取鍵。
///
/// 這個 key 以「股票代號」為粒度，而不是財報季度為粒度，讓同一支股票在
/// 不同季度的 backfill 流程中共用同一個短期 skip 狀態，避免同一天重複對
/// 明顯沒有資料的頁面發出請求。
pub fn no_valid_data_cache_key(stock_symbol: &str) -> String {
    format!("YahooProfileNoValidData:{}", stock_symbol)
}

/// 從雅虎抓取指定股票的 profile 資訊（包含財務比率、獲利能力等指標）。
///
/// 只負責 HTTP 抓取，解析工作交給 [`parse_profile_html`]（fetch/parse 分離，
/// 解析邏輯才能被 fixture 單元測試覆蓋，不需要真的連 Yahoo）。
///
/// # 參數
/// * `stock_symbol` - 股票代碼 (例如: "2330")
///
/// # 傳回值
/// 成功時傳回填充好的 `Profile` 結構，失敗時傳回包含錯誤環境資訊的 `Result`。
pub async fn visit(stock_symbol: &str) -> Result<Profile> {
    let url = format!("https://{}/quote/{}/profile", HOST, stock_symbol);
    let text = util::http::get(&url, None).await?;
    parse_profile_html(stock_symbol, &url, &text)
}

/// 解析 Yahoo profile 頁面的 HTML，取出財務比率與獲利能力指標。
///
/// 這是一個「純函式」——輸入只有 HTML 字串，不做任何網路 I/O，
/// 可用 `testdata/profile_page.html` fixture 直接驗證整頁解析流程。
///
/// # 解析流程（對照 Yahoo 頁面結構）
/// 1. 以 `BASE_SELECTOR` 定位「獲利能力」區塊（Proxy 容器下的第三個 section）。
/// 2. 從區塊的第二個 `div.D(f)` 讀出「2025 Q3」形式的年度與季度。
/// 3. 各項比率位於 CSS Grid（`table-grid`）的第 1～6 格，依序為：
///    毛利率、ROA、營益率、ROE、稅前淨利率、每股淨值。
/// 4. EPS 位於區塊的第四個 div 之下，層級不同需獨立解析。
/// 5. 防禦性檢查：年份與 EPS 同時為 0 視為「頁面存在但無有效資料」，
///    回傳 [`NoValidProfileDataError`] 讓 backfill 流程降級處理。
fn parse_profile_html(stock_symbol: &str, url: &str, text: &str) -> Result<Profile> {
    let document = Html::parse_document(text);

    // 取得主要數據區塊
    let section = document.select(&BASE_SELECTOR).next().with_context(|| {
        format!(
            "Failed to find profile section for {} at {}",
            stock_symbol, url
        )
    })?;

    let mut profile = Profile::new(stock_symbol.to_string());
    // Yahoo 的數據以 CSS Grid 呈現，這裡定義基礎路徑
    let css_base = "div.table-grid.Mb\\(20px\\).row-fit-half > div:nth-child";

    // 解析年份與季度 (例如 "2025 Q3")
    if let Some(year_and_quarter_text) = element::parse_value(&section, "div:nth-child(2).D\\(f\\)")
        && let Some(quarter_match) = REG_QUARTER.find(&year_and_quarter_text)
    {
        profile.quarter = quarter_match.as_str().to_uppercase();
        // 用 .get(0..4) 取代 [0..4]：外部頁面文字可能以中文開頭（每字 3 bytes），
        // byte 4 落在字元中間時，[0..4] 會 panic，而 .get 只會回傳 None。
        // 取不到合法年份時保持 profile.year 為 0，交由下方防禦性檢查判斷。
        if let Some(year_str) = year_and_quarter_text.get(0..4)
            && let Ok(year) = year_str.parse::<i32>()
        {
            profile.year = year;
        }
    }

    // 獲取各項財務指標 (對應 Grid 中的不同子元素)
    profile.gross_profit = parse_field(&section, css_base, 1);
    profile.return_on_assets = parse_field(&section, css_base, 2);
    profile.operating_profit_margin = parse_field(&section, css_base, 3);
    profile.return_on_equity = parse_field(&section, css_base, 4);
    profile.pre_tax_income = parse_field(&section, css_base, 5);
    profile.net_asset_value_per_share = parse_field(&section, css_base, 6);

    // 每股稅後淨利 (EPS) 位於不同的 HTML 層級，需獨立解析
    profile.earnings_per_share =
        element::parse_to_decimal(&section, "div:nth-child(4) > div:nth-child(3) > div > div");

    // 防禦性檢查：若年份為 0 且關鍵指標 EPS 也是 0，視為解析無效數據
    if profile.year == 0 && profile.earnings_per_share.is_zero() {
        return Err(NoValidProfileDataError {
            stock_symbol: stock_symbol.to_string(),
            url: url.to_string(),
        }
        .into());
    }

    Ok(profile)
}

/// 輔助函數：根據索引解析特定的 Grid 欄位數據並轉換為 `Decimal`。
fn parse_field(element: &scraper::ElementRef, base: &str, child_index: u32) -> Decimal {
    let selector = format!("{}({}) > div > div", base, child_index);
    element::parse_to_decimal(element, &selector)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    /// 以貼近真實 profile 頁形狀的 fixture 驗證整頁解析流程。
    ///
    /// 涵蓋三個層級的定位：Proxy 容器下的第三個 section、
    /// `div.D(f)` 的年度季度文字、CSS Grid 六格比率與獨立層級的 EPS。
    /// Yahoo 改版時只要用瀏覽器存一份新回應更新 fixture，
    /// 測試立即反映哪一段 selector 需要調整。
    #[test]
    fn parse_profile_html_parses_fixture() {
        // include_str! 的路徑相對於本檔案（yahoo/profile.rs）→ yahoo/testdata/。
        const FIXTURE: &str = include_str!("testdata/profile_page.html");

        let profile =
            parse_profile_html("2330", "https://example.test/quote/2330/profile", FIXTURE).unwrap();

        // 年度與季度來自「2025 Q3」文字：前 4 碼是年份，正則抓出季度。
        assert_eq!(profile.year, 2025);
        assert_eq!(profile.quarter, "Q3");
        assert_eq!(profile.stock_symbol, "2330");

        // Grid 第 1～6 格：毛利率、ROA、營益率、ROE、稅前淨利率、每股淨值。
        assert_eq!(profile.gross_profit, dec!(58.6));
        assert_eq!(profile.return_on_assets, dec!(12.9));
        assert_eq!(profile.operating_profit_margin, dec!(48.1));
        assert_eq!(profile.return_on_equity, dec!(26.3));
        assert_eq!(profile.pre_tax_income, dec!(53.2));
        assert_eq!(profile.net_asset_value_per_share, dec!(158.9));

        // EPS 位於另一個 grid 的第 3 格（層級不同，獨立解析）。
        assert_eq!(profile.earnings_per_share, dec!(15.36));
    }

    /// 頁面有 profile 區塊、但年份與 EPS 都解析不到時，
    /// 應回傳 NoValidProfileDataError（可被 is_no_valid_data_error 辨識），
    /// 讓 backfill 流程把它當「暫時無資料」降級處理，而不是一般錯誤。
    #[test]
    fn parse_profile_html_reports_no_valid_data_for_empty_section() {
        let html = r#"
            <div id="main-2-QuoteProfile-Proxy"><div>
                <section>1</section>
                <section>2</section>
                <section><h2>獲利能力</h2><div class="D(f)">尚無資料</div></section>
            </div></div>
        "#;

        let error = parse_profile_html("7811", "https://example.test/quote/7811/profile", html)
            .unwrap_err();
        assert!(is_no_valid_data_error(&error));
    }

    /// 整頁缺少 profile 區塊（載到錯誤頁）時，回傳的是一般解析錯誤，
    /// 不應被誤判為「無有效資料」的暫時性跳過。
    #[test]
    fn parse_profile_html_rejects_page_without_profile_section() {
        let error = parse_profile_html(
            "2330",
            "https://example.test/quote/2330/profile",
            "<html><body><h1>404</h1></body></html>",
        )
        .unwrap_err();

        assert!(!is_no_valid_data_error(&error));
        assert!(error.to_string().contains("profile section"));
    }

    #[test]
    fn test_is_no_valid_data_error() {
        let err: anyhow::Error = NoValidProfileDataError {
            stock_symbol: "7811".to_string(),
            url: "https://tw.stock.yahoo.com/quote/7811.TWO/profile".to_string(),
        }
        .into();

        assert!(is_no_valid_data_error(&err));
    }

    #[test]
    fn test_no_valid_data_cache_key() {
        assert_eq!(
            no_valid_data_cache_key("7811"),
            "YahooProfileNoValidData:7811"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 visit");

        match visit("2330").await {
            Ok(e) => {
                dbg!(&e);
                tracing::debug!("{:#?}", e);
            }
            Err(why) => {
                tracing::debug!("Failed to visit because {:?}", why);
            }
        }

        tracing::debug!("結束 visit");
    }
}
