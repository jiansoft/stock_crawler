use super::*;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, d).expect("合法日期")
}

/// 一筆日線列；開高低收以元為單位，`None` 表示缺值。
fn row(date: NaiveDate, open: Option<i64>, close: Option<i64>) -> HistoricalQuoteRow {
    HistoricalQuoteRow {
        date,
        opening_price: open.map(Decimal::from),
        highest_price: close.map(|close| Decimal::from(close + 2)),
        lowest_price: Some(Decimal::ZERO),
        closing_price: close.map(Decimal::from),
        change: None,
        change_range: None,
        trading_volume: None,
        transaction: None,
        trade_value: None,
        moving_average_5: None,
        moving_average_10: None,
        moving_average_20: None,
        moving_average_60: None,
        price_earning_ratio: None,
        price_to_book_ratio: None,
        record_time: None,
    }
}

fn cash_dividend(date: NaiveDate, cash: f64) -> PriceAdjustment {
    PriceAdjustment {
        date,
        cash_dividend: cash,
        stock_dividend: 0.0,
        share_ratio: 1.0,
    }
}

/// 不要求還原時不附 `adjusted`；要求時以最新一筆有成交的日線為基準，
/// 沒有成交與缺值、為 0 的價格不換算。
#[test]
fn build_quotes_attaches_adjusted_prices() {
    let rows = || {
        vec![
            row(day(30), Some(100), Some(102)),
            row(day(29), None, Some(0)),
            row(day(28), Some(100), Some(101)),
            row(day(27), Some(99), None),
        ]
    };
    let plain = build_quotes(rows(), None);
    assert!(plain.iter().all(|quote| quote.adjusted.is_none()));
    assert_eq!(plain[0].closing_price, Some(102.0));

    // 4/30 除息 1 元：前一筆有成交的 4/28 收盤 101 → 參考價 100。
    let quotes = build_quotes(rows(), Some(&[cash_dividend(day(30), 1.0)]));
    let latest = quotes[0].adjusted.as_ref().expect("有成交");
    assert_eq!(latest.factor, 1.0);
    assert_eq!(latest.closing_price, 102.0);
    assert_eq!(latest.opening_price, Some(100.0));
    assert_eq!(latest.highest_price, Some(104.0));
    assert_eq!(latest.lowest_price, None);
    assert!(quotes[1].adjusted.is_none());
    let before = quotes[2].adjusted.as_ref().expect("有成交");
    assert_eq!(before.factor, 0.9901);
    assert_eq!(before.closing_price, 100.0);
    assert_eq!(before.opening_price, Some(99.01));
    assert_eq!(before.highest_price, Some(101.98));
    assert!(quotes[3].adjusted.is_none());
    assert_eq!(quotes[3].opening_price, Some(99.0));

    assert!(build_quotes(Vec::new(), Some(&[])).is_empty());
}
