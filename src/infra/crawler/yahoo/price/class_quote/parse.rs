//! Yahoo 類股 API 的回應 DTO 與欄位解析：把單筆類股項目轉成 [`RealtimeSnapshot`]。

use std::str::FromStr;

use anyhow::{Context, Result};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::infra::cache::{PriceLimit, RealtimeSnapshot};

/// Yahoo `getClassQuotes` API 的單頁回應。
///
/// 只保留 crawler 真正需要的欄位：
/// - `list`：單頁的股票列表。
/// - `pagination`：分頁資訊。
#[derive(Debug, Default, Deserialize)]
pub(super) struct ClassQuotesResponse<'a> {
    #[serde(default)]
    #[serde(borrow)]
    pub(super) list: Vec<ClassQuoteItem<'a>>,
    #[serde(default)]
    pub(super) pagination: ClassQuotesPagination<'a>,
}

/// Yahoo 單筆類股項目。
///
/// 欄位型別盡量使用 borrowed data，降低盤中長輪詢時的暫時配置量。
#[derive(Debug, Deserialize)]
pub(super) struct ClassQuoteItem<'a> {
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
    #[serde(default, rename = "limitUpPrice")]
    limit_up_price: Option<RawNumericField<'a>>,
    #[serde(default, rename = "limitDownPrice")]
    limit_down_price: Option<RawNumericField<'a>>,
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
pub(super) struct ClassQuotesPagination<'a> {
    #[serde(default, rename = "resultsTotal")]
    pub(super) results_total: usize,
    #[serde(default, borrow, rename = "nextOffset")]
    next_offset: Option<&'a str>,
}

impl ClassQuotesPagination<'_> {
    /// 將 Yahoo 原始字串格式的 `nextOffset` 轉成數值。
    pub(super) fn next_offset(&self) -> Result<Option<usize>> {
        self.next_offset
            .map(|offset| {
                offset.parse::<usize>().with_context(|| {
                    format!("Failed to parse Yahoo class quote nextOffset: {offset}")
                })
            })
            .transpose()
    }
}

/// 將單筆 Yahoo 類股 API 項目轉成內部快照型別。
///
/// 若該筆資料沒有可辨識的股票代號，會回傳 `Ok(None)` 讓呼叫端略過。
pub(super) fn parse_class_quote_item(
    item: &ClassQuoteItem<'_>,
) -> Result<Option<(String, RealtimeSnapshot)>> {
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
    snapshot.price_limit = parse_price_limit(item);

    Ok(Some((symbol, snapshot)))
}

/// 解析漲跌停價：兩者都是 `-` 代表沒有漲跌幅限制，都是正數才是區間，其餘視為未知。
fn parse_price_limit(item: &ClassQuoteItem<'_>) -> PriceLimit {
    let (up, down) = (
        raw_value(item.limit_up_price.as_ref()),
        raw_value(item.limit_down_price.as_ref()),
    );
    if is_dash(up) && is_dash(down) {
        return PriceLimit::Unlimited;
    }
    let number = |raw: Option<&RawNumericValue<'_>>| {
        raw.and_then(|raw| decimal_from_value(raw, "", "limit").ok())
            .filter(|value| *value > Decimal::ZERO)
    };
    match (number(down), number(up)) {
        (Some(down), Some(up)) if down <= up => PriceLimit::Range { down, up },
        _ => PriceLimit::Unknown,
    }
}

/// 取出 `{ raw: ... }` 欄位的值。
fn raw_value<'a, 'b>(field: Option<&'b RawNumericField<'a>>) -> Option<&'b RawNumericValue<'a>> {
    field.and_then(|field| field.raw.as_ref())
}

/// 欄位值是否為 Yahoo 表示「無」的 `-`。
fn is_dash(raw: Option<&RawNumericValue<'_>>) -> bool {
    matches!(raw, Some(RawNumericValue::Text(text)) if text.trim() == "-")
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
    crate::core::util::text::parse_decimal(normalized, Some(vec![',', '%'])).with_context(|| {
        format!(
            "Failed to parse Yahoo {} for {}: {}",
            field_name, symbol, text
        )
    })
}

#[cfg(test)]
mod tests;
