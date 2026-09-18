//! # 年度獲利（年度財報）共用型別與解析
//!
//! 定義多個站台（MoneyDJ、FBS、HiStock、MOPS）共用的年度獲利資料結構與
//! 抓取介面 [`AnnualProfitFetcher`]，並提供「抓取」與「解析」分離的共用實作：
//! 網路層只負責取得 HTML 文字，解析則交由可離線測試的純函式處理。

use anyhow::Result;
use async_trait::async_trait;
use rust_decimal::Decimal;
use scraper::{ElementRef, Html, Selector};

use crate::{
    core::util::{self, map::Keyable, text},
    infra::crawler::CrawlerError,
};

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

/// 抓取並解析「年度獲利」頁面（MoneyDJ 與 FBS 共用相同的頁面結構）。
///
/// ## 設計說明：fetch 與 parse 分離
///
/// 這個函式刻意「很薄」——只負責發出 HTTP 請求拿到 HTML 文字，
/// 然後把文字交給純函式 [`parse_annual_profits_html`] 解析。
/// 好處是解析邏輯（最容易因站方改版而出錯、也最值得測試的部分）
/// 可以用本機的 fixture 檔案做單元測試，完全不需要連線真實網站；
/// 網路層的行為（timeout、重試）則由 `core::util::http` 統一負責。
pub(in crate::infra::crawler) async fn fetch_annual_profits(
    url: &str,
    stock_symbol: &str,
) -> Result<Vec<AnnualProfit>, CrawlerError> {
    // 保留 anyhow 錯誤鏈：message 只放頂層摘要，完整原因交給 source。
    let text = util::http::get(url, None)
        .await
        .map_err(|e| CrawlerError::Network {
            message: e.to_string(),
            source: e.into(),
        })?;

    // 拿到 HTML 文字後，解析全部交給下面的純函式。
    parse_annual_profits_html(&text, stock_symbol)
}

/// 解析「年度獲利」頁面 HTML 的純函式。
///
/// ## 為什麼抽成純函式
///
/// 「純函式」的意思是：給同樣的輸入（HTML 字串）永遠得到同樣的輸出，
/// 不做任何 I/O（不連網路、不碰資料庫）。因此測試它只需要準備一份
/// HTML 檔案（fixture，見 `testdata/annual_profit.html`），
/// 用 `include_str!` 在編譯期嵌入後直接呼叫斷言即可——
/// 測試快、穩定、離線可跑，站方改版時也能第一時間用新頁面重現問題。
///
/// ## 逐步說明
///
/// 1. `Html::parse_document`：把 HTML 文字解析成可查詢的 DOM 樹。
/// 2. CSS 選擇器 `#oMainTable > tbody > tr:nth-child(n+4)`：
///    - `#oMainTable`＝id 為 oMainTable 的表格（頁面的主資料表）。
///    - `tr:nth-child(n+4)`＝從第 4 列開始的所有列——前 3 列是表頭，不含資料。
/// 3. 逐列交給 [`parse_annual_profit`] 解析；格式不符的列（欄位不足、
///    年度不是數字等）回傳 `None` 被安靜略過，不會讓整頁解析失敗——
///    這是刻意的容錯：表尾的合計列或站方插入的廣告列不應中斷正常資料。
pub(super) fn parse_annual_profits_html(
    html: &str,
    stock_symbol: &str,
) -> Result<Vec<AnnualProfit>, CrawlerError> {
    let document = Html::parse_document(html);
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

/// 解析單一資料列（`<tr>`）為 [`AnnualProfit`]。
///
/// 回傳 `Option` 而不是 `Result`：`None` 代表「這一列不是合法資料列」
/// （例如表尾合計列、欄位不足的裝飾列），呼叫端會直接略過，
/// 讓其餘正常列的解析不受影響。
///
/// 欄位對照（`node.text()` 依序取出 `<td>` 內的文字節點）：
/// - index 0：年度（民國年，需轉西元）
/// - index 5：每股營收
/// - index 6：每股稅前淨利
/// - index 7：每股稅後盈餘（EPS）——這欄解析失敗視為整列無效，
///   因為 EPS 是這張表的核心欄位；5、6 欄壞掉則以 0 代替、保留該列。
fn parse_annual_profit(node: ElementRef, stock_symbol: &str) -> Option<AnnualProfit> {
    let tds: Vec<&str> = node.text().map(str::trim).collect();

    // 文字節點不足 8 個代表欄位不完整（表頭殘留或裝飾列），整列略過。
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

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    // === 年度獲利 HTML 解析（fixture）測試 ===
    //
    // 這組測試示範本專案的「crawler fixture 測試模式」：
    // 1. 把站方頁面的代表性內容存成 `testdata/` 下的檔案（fixture）。
    // 2. 用 `include_str!` 在「編譯期」把檔案內容嵌入測試——執行時
    //    不需要讀檔、更不需要網路，任何環境（含 CI）都能穩定執行。
    // 3. 對「純解析函式」做斷言，涵蓋正常列與各種異常列。
    // live 測試（真的連站方）仍保留 #[ignore]，只在懷疑站方改版時手動執行。

    /// 驗證 fixture 頁面中的正常資料列被完整解析，異常列被正確略過。
    #[test]
    fn parse_annual_profits_html_parses_fixture_rows() {
        // include_str! 的路徑是「相對於本原始碼檔案」（本檔位於
        // src/infra/crawler/share/，fixture 仍放在 src/infra/crawler/testdata/，
        // 故需以 ../ 回到上一層），編譯期就會檢查檔案存在，路徑錯會直接編譯失敗。
        const FIXTURE: &str = include_str!("../testdata/annual_profit.html");

        let profits = parse_annual_profits_html(FIXTURE, "2330").unwrap();

        // fixture 內有 5 個資料列候選：2 筆正常、3 筆異常（欄位不足的備註列、
        // 年度非數字的合計列、EPS 為 N/A 的列）——只有正常的 2 筆會被保留。
        assert_eq!(profits.len(), 2, "應恰好解析出 2 筆正常列: {profits:#?}");

        // 第一筆：民國 112 年 → 西元 2023 年，各欄位對應正確的 <td> 位置。
        assert_eq!(profits[0].stock_symbol, "2330");
        assert_eq!(profits[0].year, 2023);
        assert_eq!(profits[0].sales_per_share, dec!(83.36));
        assert_eq!(profits[0].profit_before_tax, dec!(37.79));
        assert_eq!(profits[0].earnings_per_share, dec!(32.34));

        // 第二筆：民國 111 年 → 西元 2022 年。
        assert_eq!(profits[1].year, 2022);
        assert_eq!(profits[1].earnings_per_share, dec!(39.20));
    }

    /// 驗證空頁面／完全不含目標表格的頁面回傳空清單而不是錯誤。
    ///
    /// 站方改版把表格 id 換掉時就會出現這種情況——解析不崩潰、
    /// 回傳空清單，由上層的資料量檢查（或 live 測試）發現異常。
    #[test]
    fn parse_annual_profits_html_returns_empty_for_unrelated_page() {
        let profits =
            parse_annual_profits_html("<html><body>改版了</body></html>", "2330").unwrap();
        assert!(profits.is_empty());
    }
}
