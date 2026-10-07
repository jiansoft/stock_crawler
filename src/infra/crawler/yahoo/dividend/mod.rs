//! # Yahoo 股利政策採集器
//!
//! 此模組負責從 Yahoo 財經抓取股票的歷年股利發放明細。
//! 資料包含現金股利、股票股利、除息/除權日以及實際發放日。
//!
//! ## 資料結構
//!
//! - `YahooDividend`：按「年度」聚合的股利列表體。
//! - `YahooDividendDetail`：單次股利發放的詳細資訊（如 2024Q1 季配息）。
//!
//! ## 解析邏輯
//!
//! - **年度判定**：優先以「除息日」或「除權日」的年份作為發放年度；若日期尚未公布，
//!   則以 `股利所屬年度 + 1` 推估發放年度。
//! - **格式化**：自動將網頁上的日期斜線 (`/`) 轉換為標準橫線 (`-`)，並保留 `尚未公布`
//!   供後續日期回補流程辨識。
//! - **效能**：使用 `Lazy` 靜態化正則與選擇器，並在內部使用 `HashMap` 進行年度聚合後再排序輸出。
//!
//! ## 檔案配置
//!
//! - 本檔：對外型別、404 錯誤與 [`visit`]。
//! - `parse`：HTML 解析與發放年度分組。
//! - `periods`：同一發放年度內期別的整理（重複期別、整年度配發併回）。

mod parse;
mod periods;

use std::{error::Error as StdError, fmt};

use anyhow::{Context, Result};
use rust_decimal::Decimal;

use self::parse::parse_dividend_html;
use crate::{core::util::http, infra::crawler::yahoo::HOST};

/// Yahoo 股利頁回 HTTP 404（頁面不存在）時的短期跳過快取秒數。
///
/// 404 幾乎都代表該證券已終止上市櫃或清算（例如上櫃終止、反向 ETF 下架），
/// 頁面不會突然長回來，因此以 30 天作為重試間隔，避免排程每 3 天
/// 重打一次注定失敗的請求並噴出誤導的錯誤日誌。
pub const PAGE_NOT_FOUND_CACHE_TTL_SECONDS: usize = 60 * 60 * 24 * 30;

/// 股票股利資料集合體
#[derive(Debug, Clone)]
pub struct YahooDividend {
    /// 股票代碼
    pub stock_symbol: String,
    /// 股利詳情列表，依「發放年度」由新到舊排序（desc）。
    ///
    /// 每個元素為 `(year, details)`：
    /// - `year`：發放年度
    /// - `details`：該年度內所有的配息記錄（例如季配息會有 4 筆）
    pub dividend: Vec<(i32, Vec<YahooDividendDetail>)>,
}

/// 單筆股利明細資訊
#[derive(Debug, Clone)]
pub struct YahooDividendDetail {
    /// 發放年度 (西元)
    pub year: i32,
    /// 股利所屬年度 (西元)
    pub year_of_dividend: i32,
    /// 期間代碼：Q1～Q4、H1～H2、M01～M12（月配）；單獨年配為空字串，混合配息年度的全年事件為 A。
    pub quarter: String,
    /// 現金股利 (元)
    pub cash_dividend: Decimal,
    /// 股票股利 (元)
    pub stock_dividend: Decimal,
    /// 除息日 (格式: YYYY-MM-DD)
    pub ex_dividend_date1: String,
    /// 除權日 (格式: YYYY-MM-DD)
    pub ex_dividend_date2: String,
    /// 現金股利發放日 (格式: YYYY-MM-DD)
    pub payable_date1: String,
    /// 股票股利發放日 (格式: YYYY-MM-DD)
    pub payable_date2: String,
}

impl YahooDividend {
    /// 建立新的 `YahooDividend` 實例。
    pub fn new(stock_symbol: String) -> Self {
        YahooDividend {
            stock_symbol,
            dividend: vec![],
        }
    }

    /// 依發放年度取得該年度的股利明細列表。
    pub fn get_dividend_by_year(&self, year: i32) -> Option<&Vec<YahooDividendDetail>> {
        self.dividend
            .iter()
            .find(|(y, _)| *y == year)
            .map(|(_, details)| details)
    }
}

/// 表示 Yahoo 對該股票代號回應 HTTP 404——整個個股頁面不存在。
///
/// 這幾乎都代表該證券已終止上市櫃或清算（實例：上櫃終止的 3089/8420、
/// 下架的反向 ETF 00699R），而不是 Yahoo 改版。獨立成型別是為了讓
/// backfill 流程能用 [`is_page_not_found_error`] 辨識並降級處理
/// （warn＋長期跳過快取），把 ERROR 留給真正的解析異常。
#[derive(Debug)]
pub struct YahooPageNotFoundError {
    /// 請求的股票代號。
    pub stock_symbol: String,
    /// 實際請求的 URL。
    pub url: String,
}

impl fmt::Display for YahooPageNotFoundError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Yahoo page for {} returned HTTP 404 at {} (證券可能已下市或清算)",
            self.stock_symbol, self.url
        )
    }
}

impl StdError for YahooPageNotFoundError {}

/// 判斷錯誤是否屬於「Yahoo 個股頁面不存在（HTTP 404）」。
///
/// 供 backfill 流程分辨「標的已下市」與「真正的抓取/解析異常」：
/// 前者降級為 warn 並長期跳過，後者維持 ERROR。
/// `downcast_ref` 會穿透 anyhow 的 context 包裝層，因此呼叫端
/// 即使先用 `with_context` 加了說明也能正確辨識。
pub fn is_page_not_found_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<YahooPageNotFoundError>().is_some()
}

/// 從 Yahoo 台股頁面抓取指定股票的股利資料。
///
/// # 參數
/// * `stock_symbol` - 股票代碼 (例如: "2330")
///
/// # 實作細節
/// 先檢查 HTTP 狀態碼：404 代表整個個股頁不存在（通常是已下市或清算的
/// 證券），回傳可被 [`is_page_not_found_error`] 辨識的專屬錯誤型別，
/// 不要與「頁面存在但解析不到股利列」混為一談——後者才可能是 Yahoo 改版。
///
/// 解析時遍歷股利列表表格，提取各項日期與金額。若該筆資料尚未公布日期，
/// 會以 `year_of_dividend + 1` 推估發放年度，讓擬定股利也能先被回補。
/// 最終結果會依照年份降序（新年度在前）排列。
pub async fn visit(stock_symbol: &str) -> Result<YahooDividend> {
    let url = format!("https://{}/quote/{}/dividend", HOST, stock_symbol);
    let response = http::get_response(&url, None).await?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(YahooPageNotFoundError {
            stock_symbol: stock_symbol.to_string(),
            url,
        }
        .into());
    }

    let text = response
        .text()
        .await
        .with_context(|| format!("Error reading Yahoo dividend page body from {url}"))?;
    parse_dividend_html(stock_symbol, &url, &text)
}

#[cfg(test)]
mod tests;
