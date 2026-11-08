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
mod tests;
