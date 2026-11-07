//! # 富邦證券（MoneyDJ）主力進出
//!
//! 資料來源為 `https://fubon-ebrokerdj.fbs.com.tw/z/zc/zco/zco_{代號}.djhtm`：
//! 最近一個交易日買超、賣超前 15 名的券商分點，以及各分點的買進、賣出、買賣超張數與佔成交比重。
//!
//! 「主力買賣超」採業界常用定義：買超前 15 名合計減賣超前 15 名合計。
//! 2026-10 實測與 BigGo 財經的主力買賣超逐張相同（2330 2026-10-05 皆為 +8,109 張）；
//! BigGo 有不少股票已停止更新，因此改用這個來源。
//!
//! ## 已知限制與陷阱
//!
//! - 頁面是 **Big5** 編碼，必須用 [`crate::core::util::http::get_use_big5`] 抓取。
//! - 交易所的分點資料（bsr）有驗證碼，無法直接抓官方。
//! - 頁面只標「最後更新日」，盤後資料更新前仍是前一個交易日的內容；呼叫端要自行比對日期。
//! - 成交清淡的股票兩側不一定都有 15 家，缺的那側是整格 `&nbsp;`；
//!   買賣相抵為 0 的分點會列在買超側。
//! - 查無代號時頁面沒有 `oMainTable` 表格，視為錯誤。

use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use once_cell::sync::Lazy;
use regex::Regex;
use rust_decimal::Decimal;
use scraper::{ElementRef, Html, Selector};

use crate::{
    core::util::{self, text},
    infra::crawler::fbs::HOST,
};

/// 從「最後更新日：2026/10/05」取出日期。
static UPDATED_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"最後更新日：(\d{4})/(\d{1,2})/(\d{1,2})").unwrap());

/// 主表格的資料列。
static ROW_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("#oMainTable tr").unwrap());

/// 資料格。
static CELL_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("td").unwrap());

/// 單一券商分點的買賣超。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerNet {
    /// 券商分點名稱（例如 `凱基-台北`）。
    pub name: String,
    /// 買進張數。
    pub buy: i64,
    /// 賣出張數。
    pub sell: i64,
    /// 買超（或賣超）張數，一律為非負值；方向由所在的那一側決定。
    pub net: i64,
    /// 佔成交比重（%），例如 `6.98` 代表 6.98%。
    pub share: Decimal,
}

/// 單一股票最近一個交易日的主力進出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerFlow {
    /// 股票代號。
    pub stock_symbol: String,
    /// 資料日期（頁面的「最後更新日」）。
    pub date: NaiveDate,
    /// 買超前幾名，依買超張數由大到小。
    pub buyers: Vec<BrokerNet>,
    /// 賣超前幾名，依賣超張數由大到小。
    pub sellers: Vec<BrokerNet>,
    /// 頁面「合計買超張數」。
    pub buy_total: i64,
    /// 頁面「合計賣超張數」。
    pub sell_total: i64,
}

impl BrokerFlow {
    /// 主力買賣超張數：合計買超減合計賣超，正值為買超。
    ///
    /// 用頁面的合計列而不是逐列加總：合計列以股數計算，逐列張數各自四捨五入，
    /// 加總會差 1～2 張（2884 2026-10-05 合計 −3,790、逐列加總 −3,789）。
    pub fn main_net(&self) -> i64 {
        self.buy_total - self.sell_total
    }

    /// 主力買賣超佔成交量的比重（%），正值為買超。
    ///
    /// 由各分點的佔成交比重加總而來，不需要另外取成交量；誤差只來自頁面的四捨五入。
    pub fn main_share(&self) -> Decimal {
        self.buyers
            .iter()
            .map(|broker| broker.share)
            .sum::<Decimal>()
            - self
                .sellers
                .iter()
                .map(|broker| broker.share)
                .sum::<Decimal>()
    }
}

/// 組出主力進出頁面的網址。
fn build_url(stock_symbol: &str) -> String {
    format!("https://{HOST}/z/zc/zco/zco_{stock_symbol}.djhtm")
}

/// 抓取單一股票最近一個交易日的主力進出。
///
/// # Errors
///
/// 請求失敗、查無代號或頁面格式不符時回傳錯誤。
pub async fn visit(stock_symbol: &str) -> Result<BrokerFlow> {
    let html = util::http::get_use_big5(&build_url(stock_symbol)).await?;
    parse(&html, stock_symbol)
}

/// 解析主力進出頁面。
fn parse(html: &str, stock_symbol: &str) -> Result<BrokerFlow> {
    let date = UPDATED_RE
        .captures(html)
        .and_then(|caps| {
            NaiveDate::from_ymd_opt(
                caps[1].parse().ok()?,
                caps[2].parse().ok()?,
                caps[3].parse().ok()?,
            )
        })
        .ok_or_else(|| anyhow!("找不到 {stock_symbol} 主力進出的最後更新日"))?;

    let document = Html::parse_document(html);
    let mut rows = document.select(&ROW_SELECTOR).peekable();
    if rows.peek().is_none() {
        return Err(anyhow!("找不到 {stock_symbol} 的主力進出表格"));
    }

    let mut buyers = Vec::new();
    let mut sellers = Vec::new();
    let mut totals = None;
    // 表頭與合計列都有 id（oScrollHead／oScrollMenu／oScrollFoot），資料列沒有。
    for row in rows {
        let cells: Vec<String> = row.select(&CELL_SELECTOR).map(cell_text).collect();
        if row.value().attr("id") == Some("oScrollFoot") {
            if let [buy_label, buy, sell_label, sell] = cells.as_slice()
                && buy_label == "合計買超張數"
                && sell_label == "合計賣超張數"
            {
                totals = Some((
                    text::parse_i64(buy, None)
                        .with_context(|| format!("解析 {stock_symbol} 合計買超失敗"))?,
                    text::parse_i64(sell, None)
                        .with_context(|| format!("解析 {stock_symbol} 合計賣超失敗"))?,
                ));
            }
            continue;
        }
        if row.value().attr("id").is_some() || cells.len() != 10 {
            continue;
        }
        if let Some(broker) = parse_side(&cells[..5])
            .with_context(|| format!("解析 {stock_symbol} 買超分點失敗: {cells:?}"))?
        {
            buyers.push(broker);
        }
        if let Some(broker) = parse_side(&cells[5..])
            .with_context(|| format!("解析 {stock_symbol} 賣超分點失敗: {cells:?}"))?
        {
            sellers.push(broker);
        }
    }

    let (buy_total, sell_total) =
        totals.ok_or_else(|| anyhow!("找不到 {stock_symbol} 主力進出的合計列"))?;

    Ok(BrokerFlow {
        stock_symbol: stock_symbol.to_string(),
        date,
        buyers,
        sellers,
        buy_total,
        sell_total,
    })
}

/// 解析一側的五個欄位：分點、買進、賣出、買賣超、佔成交比重。分點空白代表這側沒有資料。
fn parse_side(cells: &[String]) -> Result<Option<BrokerNet>> {
    let name = cells[0].as_str();
    if name.is_empty() {
        return Ok(None);
    }
    Ok(Some(BrokerNet {
        name: name.to_string(),
        buy: text::parse_i64(&cells[1], None)?,
        sell: text::parse_i64(&cells[2], None)?,
        net: text::parse_i64(&cells[3], None)?,
        share: text::parse_decimal(&cells[4], None)?,
    }))
}

/// 取出儲存格文字；`&nbsp;` 解碼後是不換行空白，`trim` 會一併去掉。
fn cell_text(cell: ElementRef<'_>) -> String {
    cell.text().collect::<String>().trim().to_string()
}

#[cfg(test)]
mod tests;
