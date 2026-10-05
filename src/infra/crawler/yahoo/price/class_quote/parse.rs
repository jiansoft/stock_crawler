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
    /// 有漲跌停價時記成區間；Yahoo 用 `-` 表示沒有漲跌幅限制（00715L）；缺欄位為未知。
    #[test]
    fn parse_class_quote_item_reads_price_limits() {
        let (_, ranged) = parse_test_item(json!({
            "systexId": "00631L",
            "price": {"raw": "39.58"},
            "limitUpPrice": {"raw": "47.54"},
            "limitDownPrice": {"raw": "31.7"}
        }))
        .unwrap()
        .unwrap();
        assert_eq!(
            ranged.price_limit,
            PriceLimit::Range {
                down: dec!(31.7),
                up: dec!(47.54)
            }
        );

        let (_, unlimited) = parse_test_item(json!({
            "systexId": "00715L",
            "price": {"raw": "70.75"},
            "limitUpPrice": {"raw": "-"},
            "limitDownPrice": {"raw": "-"}
        }))
        .unwrap()
        .unwrap();
        assert_eq!(unlimited.price_limit, PriceLimit::Unlimited);

        let (_, unknown) = parse_test_item(json!({"systexId": "2330", "price": {"raw": "2500"}}))
            .unwrap()
            .unwrap();
        assert_eq!(unknown.price_limit, PriceLimit::Unknown);
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

    #[test]
    fn parse_class_quote_item_skips_rows_without_symbol() {
        let item = json!({
            "symbolName": "缺代號",
            "price": { "raw": "88.4" }
        });

        assert!(parse_test_item(item).unwrap().is_none());
    }

    #[test]
    fn parse_class_quote_item_normalizes_missing_market_price_and_percent_text() {
        let item = json!({
            "symbol": "2317.TW",
            "symbolName": "鴻海",
            "price": { "raw": "市價" },
            "change": { "raw": "--" },
            "changePercent": "12.34%",
            "regularMarketOpen": { "raw": "-" },
            "volumeK": "1,234"
        });

        let (symbol, snapshot) = parse_test_item(item).unwrap().unwrap();

        assert_eq!(symbol, "2317");
        assert_eq!(snapshot.price, Decimal::ZERO);
        assert_eq!(snapshot.change, Decimal::ZERO);
        assert_eq!(snapshot.change_range, dec!(12.34));
        assert_eq!(snapshot.open, Decimal::ZERO);
        assert_eq!(snapshot.volume, dec!(1234));
    }

    #[test]
    fn parse_class_quote_item_reports_malformed_decimal() {
        let item = json!({
            "symbol": "2330.TW",
            "price": { "raw": "not-a-number" }
        });

        let error = parse_test_item(item).unwrap_err().to_string();

        assert!(error.contains("Failed to parse Yahoo price for 2330"));
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

    #[test]
    fn next_offset_reports_malformed_values() {
        let pagination = ClassQuotesPagination {
            results_total: 89,
            next_offset: Some("next-page"),
        };

        let error = pagination.next_offset().unwrap_err().to_string();

        assert!(error.contains("Failed to parse Yahoo class quote nextOffset"));
    }
}
