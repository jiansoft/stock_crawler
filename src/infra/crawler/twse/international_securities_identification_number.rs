use anyhow::Result;
use chrono::Local;
use scraper::{Html, Selector};

use crate::{
    core::declare::StockExchangeMarket,
    core::util::{self, datetime::Weekend},
    infra::crawler::twse,
};

const REQUIRED_CATEGORIES: [&str; 4] = ["股票", "特別股", "普通股", TDR_CATEGORY];

/// 臺灣存託憑證的類別標頭；這類證券的產業別欄是空的，改用「存託憑證」產業。
const TDR_CATEGORY: &str = "臺灣存託憑證(TDR)";

/// twse 國際證券識別碼
#[derive(Debug, Clone)]
pub struct InternationalSecuritiesIdentificationNumber {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 國際證券識別碼（ISIN）。
    pub isin_code: String,
    /// 上市日期。
    pub listing_date: String,
    /// 產業分類名稱。
    pub industry: String,
    /// CFI Code。
    pub cfi_code: String,
    /// 交易市場。
    pub market: StockExchangeMarket,
}

/// 調用  twse API 取得台股國際證券識別碼
/// 上市:2 上櫃︰4 興櫃︰5
pub async fn visit(
    mode: StockExchangeMarket,
) -> Result<Vec<InternationalSecuritiesIdentificationNumber>> {
    if Local::now().is_weekend() {
        return Ok(Vec::new());
    }

    Ok(parse_isin_html(&fetch_page(mode).await?, mode))
}

/// 取得指定市場的 ISIN 名冊頁（Big5 編碼）。
async fn fetch_page(mode: StockExchangeMarket) -> Result<String> {
    let url = format!(
        "https://isin.{}/isin/C_public.jsp?strMode={}",
        twse::HOST,
        mode.serial()
    );
    util::http::get_use_big5(&url).await
}

/// 調用 twse API 取得指定市場「全部類別」的現行掛牌證券代號名冊。
///
/// 與 [`visit`] 的差別：[`visit`] 只保留股票／特別股／TDR 等類別供股票主檔
/// 回補使用；這裡**不過濾類別**（ETF、ETN、受益證券等全數納入），
/// 專供「下市差集比對」使用——名冊必須涵蓋資料庫可能存在的所有證券類型，
/// 否則不在名冊類別內的證券（例如 ETF）會被誤判為已下市。
///
/// 注意：與 [`visit`] 相同，週末直接回傳空集合（來源不更新），
/// 呼叫端應避免在週末執行差集比對。
pub async fn visit_all_listed_symbols(
    mode: StockExchangeMarket,
) -> Result<std::collections::HashSet<String>> {
    if Local::now().is_weekend() {
        return Ok(std::collections::HashSet::new());
    }

    Ok(parse_all_symbols_html(&fetch_page(mode).await?))
}

/// 解析 TWSE ISIN 頁面，取出「全部類別」的證券代號集合。
///
/// 這是一個「純函式」——輸入只有 HTML 字串，不做任何網路 I/O。
/// 與 [`parse_isin_html`] 共用同一頁面結構，但：
/// - 不過濾證券類別（類別標頭列直接跳過）。
/// - 代號切割放寬為「第一個全形空格前的部分」，寧可多收也不漏收——
///   名冊愈完整，下市差集的誤判風險愈低。
fn parse_all_symbols_html(html: &str) -> std::collections::HashSet<String> {
    let mut result = std::collections::HashSet::with_capacity(4096);
    let document = Html::parse_document(html);

    let Ok(selector) = Selector::parse("body > table.h4 > tbody > tr") else {
        return result;
    };

    for node in document.select(&selector).skip(1) {
        let tds: Vec<&str> = node.text().map(str::trim).collect();
        // 類別標頭列只有 2 欄（例如「股票」「ETF」），不含證券資料。
        if tds.len() == 2 {
            continue;
        }

        // 第一欄格式為「代號\u{3000}名稱」；取全形空格前的代號。
        if let Some((symbol, _name)) = tds.first().and_then(|first| first.split_once('\u{3000}')) {
            let symbol = symbol.trim();
            if !symbol.is_empty() {
                result.insert(symbol.to_string());
            }
        }
    }

    result
}

/// 解析 TWSE ISIN HTML 頁面，回傳符合條件的識別碼列表。
///
/// 只保留 `REQUIRED_CATEGORIES` 中的證券類別（股票、特別股、普通股、TDR）。
/// 每列第一欄格式為「代號\u{3000}名稱」（全形空格分隔）；欄位數 5 表示省略產業分類。
fn parse_isin_html(
    html: &str,
    mode: StockExchangeMarket,
) -> Vec<InternationalSecuritiesIdentificationNumber> {
    let mut result: Vec<InternationalSecuritiesIdentificationNumber> = Vec::with_capacity(4096);
    let document = Html::parse_document(html);

    let Ok(selector) = Selector::parse("body > table.h4 > tbody > tr") else {
        return result;
    };

    let mut is_required_category = false;
    let mut is_tdr = false;
    for node in document.select(&selector).skip(1) {
        let tds: Vec<&str> = node.text().map(str::trim).collect();
        if tds.len() == 2 {
            is_required_category = REQUIRED_CATEGORIES.contains(&tds[0].trim());
            is_tdr = tds[0].trim() == TDR_CATEGORY;
            continue;
        }

        if !is_required_category {
            continue;
        }

        let split: Vec<&str> = tds[0].split('\u{3000}').collect();
        if split.len() != 2 {
            continue;
        }

        // 產業別欄空白時（TDR、特別股）只會有 5 欄；TDR 歸入「存託憑證」，其餘為「未分類」
        // （特別股在 ACL 轉換時改用普通股的產業）。
        let missing_industry = || if is_tdr { "存託憑證" } else { "未分類" }.to_string();
        let (industry, cfi_code) = match tds.len() {
            5 => (missing_industry(), tds[4].to_owned()),
            6 => (tds[4].to_owned(), tds[5].to_owned()),
            _ => (missing_industry(), String::new()),
        };

        result.push(InternationalSecuritiesIdentificationNumber {
            stock_symbol: split[0].trim().to_owned(),
            name: split[1].to_owned(),
            isin_code: tds[1].to_owned(),
            listing_date: tds[2].to_owned(),
            industry,
            cfi_code,
            market: mode,
        });
    }

    result
}

#[cfg(test)]
mod tests;
