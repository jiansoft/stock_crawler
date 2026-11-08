//! 共用快取查詢的測試。

use crate::infra::database::table::revenue;

use super::super::share::Share;

fn make_test_revenue(stock_symbol: &str, date: i64) -> revenue::Revenue {
    let mut r = revenue::Revenue::new();
    r.stock_symbol = stock_symbol.to_string();
    r.date = date;
    r
}

#[test]
fn test_set_last_revenues_creates_new_month_bucket() {
    let share = Share::new();
    share.set_last_revenues(make_test_revenue("2330", 202501));
    assert!(share.last_revenues_contains_key(202501, "2330"));
}

#[test]
fn static_lookup_tables_have_known_defaults_and_fallbacks() {
    let share = Share::new();
    let listed = share.get_exchange_market(2).unwrap();
    assert_eq!(listed.stock_exchange_market_id, 2);
    assert!(share.get_exchange_market(999).is_none());
    assert_eq!(share.get_industry_id("水泥工業"), Some(1));
    assert_eq!(share.get_industry_id("不存在產業"), Some(99));
    assert_eq!(share.get_industry_name(99), Some("未分類".to_string()));
    assert_eq!(share.get_industry_name(100), None);
}

#[test]
fn current_ip_round_trips_without_loading_external_sources() {
    let share = Share::new();
    // 初始狀態必須是 None（尚未取得），loader 才會真的去抓公網 IP；
    // 舊版初始值為空字串（Some("")），導致抓取條件永遠不成立。
    assert_eq!(share.get_current_ip(), None);
    share.set_current_ip("203.0.113.1".to_string());
    assert_eq!(share.get_current_ip(), Some("203.0.113.1".to_string()));
}

#[tokio::test]
async fn test_set_and_get_stock_index_round_trips() {
    use crate::domain::market_index::MarketIndex;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    let share = Share::new();
    let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
    let index = MarketIndex::new(
        "TAIEX".to_string(),
        date,
        Decimal::from(20000),
        Decimal::from(100),
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
    );

    let key = "2025-06-01-TAIEX".to_string();
    assert!(share.get_stock_index(&key).is_none());

    share.set_stock_index(key.clone(), index).await;

    let got = share.get_stock_index(&key).unwrap();
    assert_eq!(got.category, "TAIEX");
    assert_eq!(got.date, date);
    assert_eq!(got.index, Decimal::from(20000));
}
