//! # TPEx 上櫃除權除息預告表採集器
//!
//! 資料來源為櫃買中心 OpenAPI `openapi/v1/tpex_exright_prepost`，對應網頁上的
//! 「除權除息預告表」（`announce/market/ex/announce.html`）。
//!
//! ## 與 TWSE 的差異
//!
//! 欄位語意與 [`crate::infra::crawler::twse::ex_dividend_announcement`] 相同，
//! 只有欄位名稱與類別字串不同（TPEx 用 `除息`／`除權`／`除權息`，TWSE 用 `息`／`權`／`權息`），
//! 因此兩邊都轉譯成共用的 [`ExDividendAnnouncement`]，上層流程不必分辨市場。
//!
//! 另一個差別是空值表示法：TPEx 在「沒有配股」時給的是 `0.00000000` 而不是空字串，
//! 所以 0 在這裡是真正的 0；仍以空字串代表未公布。
//!
//! 「除權」可能只是現金增資（無償配股率 0、現增配股率大於 0），這類資料列沒有股利，
//! 會由 [`classify_ex_dividend`] 辨識後丟棄。

use anyhow::Result;
use serde::Deserialize;

use crate::{
    core::{
        declare::StockExchangeMarket,
        util::{self, datetime},
    },
    infra::crawler::{
        share::{ExDividendAnnouncement, classify_ex_dividend},
        tpex,
    },
};

/// TPEx OpenAPI `tpex_exright_prepost` 的原始資料列。
///
/// 欄位名稱沿用官方的拼字（含 `ExRrights` 這個官方 typo），
/// 以免因為「看起來像錯字」而改壞對應關係。
#[derive(Deserialize, Debug, Clone)]
struct TpexExRightPrepostRaw {
    /// 除權除息日期（民國 `YYYMMDD`）。
    #[serde(rename = "ExRrightsExDividendDate")]
    date: String,
    /// 股票代號。
    #[serde(rename = "SecuritiesCompanyCode")]
    code: String,
    /// 公司名稱。
    #[serde(rename = "CompanyName")]
    name: String,
    /// 除權息類別：`除息`、`除權`、`除權息`。
    #[serde(rename = "ExRrightsExDividend")]
    ex_dividend: String,
    /// 無償配股率（股/股）。
    #[serde(rename = "StockDividendRatio")]
    stock_dividend_ratio: String,
    /// 現金增資認購配股率（股/股）；非現增事件為 `0.00000000`。
    #[serde(rename = "SubscriptionRatioToNewSharesIssued", default)]
    subscription_ratio: String,
    /// 現金股利（元/股）。
    #[serde(rename = "CashDividend")]
    cash_dividend: String,
}

/// 取得上櫃股票的除權除息預告表。
///
/// 與上市版一樣是**全市場**的預告事件，一次請求即可比對出漏抓的事件。
///
/// # 錯誤
///
/// 當 HTTP 請求或 JSON 反序列化失敗時回傳錯誤。
pub async fn visit() -> Result<Vec<ExDividendAnnouncement>> {
    let url = format!(
        "https://{host}/openapi/v1/tpex_exright_prepost",
        host = tpex::HOST
    );
    let data = util::http::get_json::<Vec<TpexExRightPrepostRaw>>(&url).await?;

    Ok(parse_announcements(data))
}

/// 將 TPEx 原始資料列轉成共用的 [`ExDividendAnnouncement`]。
///
/// 這是純函式，可用 `testdata/ex_dividend_prepost.json` fixture 直接驗證。
/// 純現金增資除權（沒有任何股利）的資料列會丟棄。
fn parse_announcements(rows: Vec<TpexExRightPrepostRaw>) -> Vec<ExDividendAnnouncement> {
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
                market: StockExchangeMarket::OverTheCounter,
            })
        })
        .collect()
}

/// 解析可能為「未公布」的數值欄位；空字串或 `-` 視為未公布。
fn parse_optional_decimal(value: &str) -> Option<rust_decimal::Decimal> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return None;
    }

    util::text::parse_decimal(trimmed, None).ok()
}

#[cfg(test)]
mod tests;
