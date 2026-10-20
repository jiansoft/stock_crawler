//! 各快取的整批替換與單筆更新（不需外部服務）。

use super::*;

fn make_test_revenue(stock_symbol: &str, date: i64) -> revenue::Revenue {
    let mut r = revenue::Revenue::new();
    r.stock_symbol = stock_symbol.to_string();
    r.date = date;
    r
}

fn make_test_index(category: &str, date: NaiveDate) -> MarketIndex {
    MarketIndex::new(
        category.to_string(),
        date,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
    )
}

#[test]
fn test_replace_indices_cache_overwrites_old_entries() {
    let share = Share::new();
    let old_date = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let new_date = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();

    share.replace_indices_cache(vec![make_test_index("TAIEX", old_date)]);
    assert!(share.get_stock_index("2025-01-01-TAIEX").is_some());

    share.replace_indices_cache(vec![make_test_index("TAIEX", new_date)]);

    assert!(share.get_stock_index("2025-02-01-TAIEX").is_some());
    assert!(share.get_stock_index("2025-01-01-TAIEX").is_none());
}

#[test]
fn test_replace_last_revenues_cache_overwrites_old_months() {
    let share = Share::new();

    share.replace_last_revenues_cache(vec![
        make_test_revenue("2330", 202501),
        make_test_revenue("2317", 202502),
    ]);
    assert!(share.last_revenues_contains_key(202501, "2330"));
    assert!(share.last_revenues_contains_key(202502, "2317"));

    share.replace_last_revenues_cache(vec![make_test_revenue("2454", 202503)]);

    assert!(!share.last_revenues_contains_key(202501, "2330"));
    assert!(!share.last_revenues_contains_key(202502, "2317"));
    assert!(share.last_revenues_contains_key(202503, "2454"));
}

#[tokio::test]
async fn replace_stocks_cache_controls_stock_lookup_and_contains() {
    let share = Share::new();
    let stock = crate::domain::registry::entity::Stock::register(
        "2330".to_string(),
        "台積電".to_string(),
        0,
        0,
    );

    share.replace_stocks_cache(vec![stock]);

    assert!(share.stock_contains_key("2330"));
    assert!(!share.stock_contains_key("2317"));
    assert_eq!(share.get_stock("2330").await.unwrap().name(), "台積電");
    assert!(share.get_stock("2317").await.is_none());
}

#[tokio::test]
async fn test_replace_last_trading_day_quotes_cache_overwrites() {
    use crate::infra::database::table::last_daily_quotes::LastDailyQuotes;
    use rust_decimal_macros::dec;

    let share = Share::new();

    let mut q1 = LastDailyQuotes::new();
    q1.stock_symbol = "2330".to_string();
    q1.closing_price = dec!(500);
    q1.date = NaiveDate::from_ymd_opt(2025, 1, 2).unwrap();

    let mut q2 = LastDailyQuotes::new();
    q2.stock_symbol = "2317".to_string();
    q2.closing_price = dec!(100);

    share.replace_last_trading_day_quotes_cache(vec![q1, q2]);

    assert!(share.get_stock_last_price("2330").await.is_some());
    assert!(share.get_stock_last_price("2317").await.is_some());
    assert!(share.get_stock_last_price("2454").await.is_none());

    let mut q3 = LastDailyQuotes::new();
    q3.stock_symbol = "2454".to_string();
    q3.closing_price = dec!(200);

    share.replace_last_trading_day_quotes_cache(vec![q3]);

    assert!(share.get_stock_last_price("2330").await.is_none());
    assert!(share.get_stock_last_price("2454").await.is_some());
}

#[tokio::test]
async fn test_set_stock_last_price_updates_existing_entry() {
    use crate::domain::quote::entity::DailyQuote;
    use crate::infra::database::table::last_daily_quotes::LastDailyQuotes;
    use rust_decimal_macros::dec;

    let share = Share::new();

    let mut q = LastDailyQuotes::new();
    q.stock_symbol = "2330".to_string();
    q.closing_price = dec!(500);
    q.date = NaiveDate::from_ymd_opt(2025, 1, 2).unwrap();

    share.replace_last_trading_day_quotes_cache(vec![q]);

    let before = share.get_stock_last_price("2330").await.unwrap();
    assert_eq!(before.closing_price, dec!(500));

    let new_date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
    // 初始化 DailyQuote 並覆寫部分預設欄位
    let dq = DailyQuote {
        stock_symbol: "2330".to_string(),
        closing_price: dec!(620),
        date: new_date,
        ..DailyQuote::default()
    };

    share.set_stock_last_price(&dq).await;

    let after = share.get_last_trading_day_quotes("2330").await.unwrap();
    assert_eq!(after.closing_price, dec!(620));
    assert_eq!(after.date, new_date);
}

#[test]
fn test_replace_quote_history_records_cache_overwrites() {
    use crate::domain::quote::entity::QuoteHistoryRecord;
    use rust_decimal_macros::dec;

    let share = Share::new();

    let mut r1 = QuoteHistoryRecord::new("2330".to_string());
    r1.maximum_price = dec!(600);
    r1.minimum_price = dec!(400);

    share.replace_quote_history_records_cache(vec![r1]);

    {
        let got = share.quote_history_records.read().unwrap();
        let r = got.get("2330").unwrap();
        assert_eq!(r.maximum_price, dec!(600));
        assert_eq!(r.minimum_price, dec!(400));
    }

    let r2 = QuoteHistoryRecord::new("2454".to_string());
    share.replace_quote_history_records_cache(vec![r2]);

    let got = share.quote_history_records.read().unwrap();
    assert!(got.get("2330").is_none());
    assert!(got.get("2454").is_some());
}
