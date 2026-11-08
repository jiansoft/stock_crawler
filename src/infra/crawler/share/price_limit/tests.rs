//! 交易所公告的漲跌停價（上市 TWT84U、上櫃每日收盤行情的次日漲跌停價）的測試。

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
