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

use anyhow::{anyhow, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use rust_decimal::Decimal;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use crate::{crawler::yahoo::HOST, util, util::http::element};

/// 用於解析季度（如 Q1, Q2）的正則表達式
static REG_QUARTER: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)q\d").expect("Failed to compile quarter regex"));

/// 個股基本資料區塊的主要選擇器
static BASE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse("#main-2-QuoteProfile-Proxy > div > section:nth-child(3)")
        .expect("Failed to parse base profile selector")
});

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

/// 從雅虎抓取指定股票的 profile 資訊（包含財務比率、獲利能力等指標）。
///
/// # 參數
/// * `stock_symbol` - 股票代碼 (例如: "2330")
///
/// # 傳回值
/// 成功時傳回填充好的 `Profile` 結構，失敗時傳回包含錯誤環境資訊的 `Result`。
pub async fn visit(stock_symbol: &str) -> Result<Profile> {
    let url = format!("https://{}/quote/{}/profile", HOST, stock_symbol);
    let text = util::http::get(&url, None).await?;
    let document = Html::parse_document(&text);

    // 取得主要數據區塊
    let section = document
        .select(&BASE_SELECTOR)
        .next()
        .with_context(|| format!("Failed to find profile section for {} at {}", stock_symbol, url))?;

    let mut profile = Profile::new(stock_symbol.to_string());
    // Yahoo 的數據以 CSS Grid 呈現，這裡定義基礎路徑
    let css_base = "div.table-grid.Mb\\(20px\\).row-fit-half > div:nth-child";

    // 解析年份與季度 (例如 "2025 Q3")
    if let Some(year_and_quarter_text) = element::parse_value(&section, "div:nth-child(2).D\\(f\\)") {
        if let Some(quarter_match) = REG_QUARTER.find(&year_and_quarter_text) {
            profile.quarter = quarter_match.as_str().to_uppercase();
            if let Ok(year) = year_and_quarter_text[0..4].parse::<i32>() {
                profile.year = year;
            }
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
        return Err(anyhow!("Parsed profile for {} contains no valid data. Site structure might have changed.", stock_symbol));
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
    use crate::logging;
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match visit("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("{:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}

