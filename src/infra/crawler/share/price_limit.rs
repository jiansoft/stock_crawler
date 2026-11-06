//! 交易所公告的漲跌停價（上市 TWT84U、上櫃每日收盤行情的次日漲跌停價）。

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::infra::cache::PriceLimit;

/// 一份交易所漲跌停價資料。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangePriceLimits {
    /// 資料日期：上市是這份漲跌停價適用的交易日；上櫃是收盤行情的日期（漲跌停價適用於下一個交易日）。
    pub date: NaiveDate,
    /// 代號與漲跌幅限制；無法解析的列已略過。
    pub limits: Vec<(String, PriceLimit)>,
}

impl ExchangePriceLimits {
    /// 從一列文字欄位取出代號與漲跌停價；代號空白或價格無法解析時回傳 `None`。
    pub(crate) fn row(
        row: &[serde_json::Value],
        symbol: usize,
        down: usize,
        up: usize,
    ) -> Option<(String, PriceLimit)> {
        let text = |index: usize| row.get(index).and_then(serde_json::Value::as_str);
        let price =
            |index: usize| -> Option<Decimal> { text(index)?.trim().replace(',', "").parse().ok() };
        let symbol = text(symbol)?.trim();
        if symbol.is_empty() {
            return None;
        }
        match PriceLimit::from_exchange_prices(price(down)?, price(up)?) {
            PriceLimit::Unknown => None,
            limit => Some((symbol.to_owned(), limit)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(text: &str) -> Decimal {
        text.parse().expect("decimal")
    }

    /// 0.01／9,999.95 是無限制；正常區間照收；跌停高於漲停或非正數視為未知，該列略過。
    #[test]
    fn exchange_prices_map_to_limits() {
        assert_eq!(
            PriceLimit::from_exchange_prices(dec("0.01"), dec("9999.95")),
            PriceLimit::Unlimited
        );
        assert_eq!(
            PriceLimit::from_exchange_prices(dec("8.81"), dec("10.75")),
            PriceLimit::Range {
                down: dec("8.81"),
                up: dec("10.75")
            }
        );
        assert_eq!(
            PriceLimit::from_exchange_prices(dec("11"), dec("10")),
            PriceLimit::Unknown
        );
        assert_eq!(
            PriceLimit::from_exchange_prices(Decimal::ZERO, dec("10")),
            PriceLimit::Unknown
        );

        let row = serde_json::json!(["1234", "0", "11", "10"]);
        let row = row.as_array().expect("array");
        assert_eq!(ExchangePriceLimits::row(row, 0, 2, 3), None);
        assert_eq!(ExchangePriceLimits::row(row, 9, 2, 3), None);
    }
}
