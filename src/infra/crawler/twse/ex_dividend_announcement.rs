//! # TWSE 上市除權除息預告表採集器
//!
//! 資料來源為證交所 OpenAPI `exchangeReport/TWT48U_ALL`，對應網頁上的
//! 「除權除息預告表」（`announcement/ex-right/twt48u.html`）。
//!
//! ## 為什麼用 OpenAPI 而不是 rwd 版
//!
//! 同一份資料的 `rwd/zh/exRight/TWT48U` 端點會把「待公告實際收益分配金額」
//! 這類提示以 HTML 片段（`<p style=...>`）塞進現金股利欄位，解析前得先清一輪標籤；
//! OpenAPI 版在同樣情況下直接給空字串，語意乾淨且不必處理 HTML。
//!
//! ## 欄位注意事項
//!
//! - `StockDividendRatio` 是**無償配股率**；`SubscriptionRatio` 是現金增資認購配股率，
//!   兩者意義完全不同，後者不能併入股票股利。現增配股率只用來辨識「權」是現增還是配股
//!   （見 [`classify_ex_dividend`]），純現增的資料列不是股利事件，會直接丟棄。
//! - 空字串代表「未公布或不適用」，會轉成 `None`，不可視為 0。

use anyhow::Result;
use serde::Deserialize;

use crate::{
    core::{
        declare::StockExchangeMarket,
        util::{self, datetime},
    },
    infra::crawler::{
        share::{ExDividendAnnouncement, classify_ex_dividend},
        twse,
    },
};

/// TWSE OpenAPI `TWT48U_ALL` 的原始資料列。
///
/// 只宣告本模組會用到的欄位；`serde` 預設忽略其餘欄位，
/// 交易所日後增欄不會讓反序列化失敗。
#[derive(Deserialize, Debug, Clone)]
struct Twt48uRaw {
    /// 除權除息日期（民國 `YYYMMDD`）。
    #[serde(rename = "Date")]
    date: String,
    /// 股票代號。
    #[serde(rename = "Code")]
    code: String,
    /// 股票名稱。
    #[serde(rename = "Name")]
    name: String,
    /// 除權息類別：`息`、`權`、`權息`。
    #[serde(rename = "Exdividend")]
    ex_dividend: String,
    /// 無償配股率（股/股）。
    #[serde(rename = "StockDividendRatio")]
    stock_dividend_ratio: String,
    /// 現金增資認購配股率（股/股）；非現增事件為空字串。
    #[serde(rename = "SubscriptionRatio", default)]
    subscription_ratio: String,
    /// 現金股利（元/股）。
    #[serde(rename = "CashDividend")]
    cash_dividend: String,
}

/// 取得上市股票的除權除息預告表。
///
/// 回傳的是**全市場**尚未除權息的預告事件（不需帶查詢參數），
/// 因此一次請求就能與資料庫比對出漏抓的事件。
///
/// # 錯誤
///
/// 當 HTTP 請求或 JSON 反序列化失敗時回傳錯誤。
pub async fn visit() -> Result<Vec<ExDividendAnnouncement>> {
    let url = format!(
        "https://openapi.{host}/v1/exchangeReport/TWT48U_ALL",
        host = twse::HOST
    );
    let data = util::http::get_json::<Vec<Twt48uRaw>>(&url).await?;

    Ok(parse_announcements(data))
}

/// 將 TWSE 原始資料列轉成共用的 [`ExDividendAnnouncement`]。
///
/// 這是純函式，可用 `testdata/ex_dividend_twt48u.json` fixture 直接驗證。
/// 日期無法解析（格式異常）的資料列會被丟棄，避免把壞資料帶進後續流程；
/// 純現金增資除權（沒有任何股利）的資料列也會丟棄。
fn parse_announcements(rows: Vec<Twt48uRaw>) -> Vec<ExDividendAnnouncement> {
    rows.into_iter()
        .filter_map(|row| {
            let ex_date = datetime::parse_taiwan_date_short(row.date.trim())?;
            let stock_dividend_ratio = parse_optional_decimal(&row.stock_dividend_ratio);
            let (is_cash, is_stock) = classify_ex_dividend(
                &row.ex_dividend,
                stock_dividend_ratio,
                parse_optional_decimal(&row.subscription_ratio),
            );
            if !is_cash && !is_stock {
                return None;
            }

            Some(ExDividendAnnouncement {
                stock_symbol: row.code.trim().to_string(),
                name: row.name.trim().to_string(),
                ex_date,
                is_cash,
                is_stock,
                cash_dividend: parse_optional_decimal(&row.cash_dividend),
                stock_dividend_ratio,
                market: StockExchangeMarket::Listed,
            })
        })
        .collect()
}

/// 解析可能為「未公布」的數值欄位。
///
/// 空字串、`-` 或無法解析的內容都視為尚未公布（`None`），
/// 這與「公布了但金額為 0」是不同語意，不可混為一談。
fn parse_optional_decimal(value: &str) -> Option<rust_decimal::Decimal> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return None;
    }

    util::text::parse_decimal(trimmed, None).ok()
}

#[cfg(test)]
mod tests;
