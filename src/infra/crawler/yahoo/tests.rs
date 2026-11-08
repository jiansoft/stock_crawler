//! Yahoo 財經採集模組的測試。

use super::*;

/// 依 `exchange + sector_id` 從預先整理的類股清單中找出目標類股。
fn find_category(
    exchange: YahooClassExchange,
    sector_id: u32,
) -> Option<&'static YahooClassCategory> {
    class_categories(exchange)
        .iter()
        .find(|category| category.sector_id == sector_id)
}

/// 驗證三大市場的 Yahoo 交易所代碼與中文標籤是否正確。
#[test]
fn test_exchange_code_and_label() {
    assert_eq!(YahooClassExchange::Listed.code(), "TAI");
    assert_eq!(YahooClassExchange::Listed.label(), "上市");
    assert_eq!(YahooClassExchange::OverTheCounter.code(), "TWO");
    assert_eq!(YahooClassExchange::OverTheCounter.label(), "上櫃");
    assert_eq!(YahooClassExchange::Emerging.code(), "OES");
    assert_eq!(YahooClassExchange::Emerging.label(), "興櫃");
}

/// 驗證三大市場預先整理好的類股數量沒有意外變動。
#[test]
fn test_class_category_counts() {
    assert_eq!(LISTED_CLASS_CATEGORIES.len(), 42);
    assert_eq!(OTC_CLASS_CATEGORIES.len(), 35);
    assert_eq!(EMERGING_CLASS_CATEGORIES.len(), 27);
}

/// 驗證認購 / 認售 / 指數類等分類會在常數表上直接標成不採集。
#[test]
fn test_collect_enabled_marks_disabled_categories() {
    let listed_call = find_category(YahooClassExchange::Listed, 31).unwrap();
    let listed_put = find_category(YahooClassExchange::Listed, 32).unwrap();
    let listed_index = find_category(YahooClassExchange::Listed, 33).unwrap();
    let listed_bull = find_category(YahooClassExchange::Listed, 51).unwrap();
    let listed_bear = find_category(YahooClassExchange::Listed, 52).unwrap();
    let otc_call = find_category(YahooClassExchange::OverTheCounter, 165).unwrap();
    let otc_put = find_category(YahooClassExchange::OverTheCounter, 166).unwrap();
    let otc_index = find_category(YahooClassExchange::OverTheCounter, 33).unwrap();
    let listed_semiconductor = find_category(YahooClassExchange::Listed, 40).unwrap();

    assert!(!listed_call.collect_enabled);
    assert!(!listed_put.collect_enabled);
    assert!(!listed_index.collect_enabled);
    assert!(!listed_bull.collect_enabled);
    assert!(!listed_bear.collect_enabled);
    assert!(!otc_call.collect_enabled);
    assert!(!otc_put.collect_enabled);
    assert!(!otc_index.collect_enabled);
    assert!(listed_semiconductor.collect_enabled);
}

/// 驗證幾個代表性 `sector_id` 仍對應到預期的類股名稱。
#[test]
fn test_known_category_sector_ids() {
    assert_eq!(
        find_category(YahooClassExchange::Listed, 1).map(|category| category.name),
        Some("水泥")
    );
    assert_eq!(
        find_category(YahooClassExchange::OverTheCounter, 122).map(|category| category.name),
        Some("食品")
    );
    assert_eq!(
        find_category(YahooClassExchange::Emerging, 99327).map(|category| category.name),
        Some("文化創意")
    );
}
