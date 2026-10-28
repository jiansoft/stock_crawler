use std::collections::HashMap;

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::super::realtime::RealtimeSnapshot;
use super::super::share::Share;

#[test]
fn test_set_stock_snapshot_price_preserves_existing_fields() {
    let share = Share::new();
    let mut snapshot = RealtimeSnapshot::new("2330".to_string(), Decimal::new(998, 0));
    snapshot.name = "台積電".to_string();
    snapshot.source_site = "HiStock".to_string();
    snapshot.change = Decimal::new(5, 0);

    let mut snapshots = HashMap::new();
    snapshots.insert("2330".to_string(), snapshot);
    share.set_stock_snapshots(snapshots);

    share.set_stock_snapshot_price("2330".to_string(), Decimal::new(1000, 0));

    let updated = share.get_stock_snapshot("2330").unwrap();
    assert_eq!(updated.price, Decimal::new(1000, 0));
    assert_eq!(updated.name, "台積電");
    assert_eq!(updated.source_site, "HiStock");
    assert_eq!(updated.change, Decimal::new(5, 0));
}

/// 同一檔同一站的異常價格重複出現時，每一輪都要過濾並保留舊值
/// （第一次記 warn、之後記 debug，只影響日誌層級）。
#[test]
fn repeated_abnormal_price_keeps_previous_snapshot() {
    let share = Share::new();
    let mut good = RealtimeSnapshot::new("79968".to_string(), dec!(78.5));
    good.last_close = dec!(75.4);
    good.source_site = "HiStock".to_string();
    share.set_stock_snapshots(HashMap::from([("79968".to_string(), good)]));

    for _ in 0..2 {
        let mut bad = RealtimeSnapshot::new("79968".to_string(), dec!(33));
        bad.last_close = dec!(75.4);
        bad.source_site = "HiStock".to_string();
        share.set_stock_snapshots(HashMap::from([("79968".to_string(), bad)]));

        assert_eq!(share.get_stock_snapshot("79968").unwrap().price, dec!(78.5));
    }

    share.set_stock_snapshot_price_with_source("79968".to_string(), dec!(1.08), "CnYes");
    share.set_stock_snapshot_price("79968".to_string(), dec!(1.08));
    assert_eq!(share.get_stock_snapshot("79968").unwrap().price, dec!(78.5));
}

/// `all_stock_snapshots` 必須把快取內每一筆都完整複製出來（供全市場排行
/// 掃描），且在快取為空（非交易時段）時回傳空 `Vec` 而非 panic。
#[test]
fn all_stock_snapshots_returns_every_cached_entry() {
    let share = Share::new();
    // 空快取代表非交易時段：必須是空 Vec，呼叫端才能據此切換資料來源。
    assert!(share.all_stock_snapshots().is_empty());

    let mut snapshots = HashMap::new();
    for symbol in ["2330", "2317", "6446"] {
        let mut snapshot = RealtimeSnapshot::new(symbol.to_string(), Decimal::new(100, 0));
        // last_close 與 price 相同可避開 `is_valid_price` 的漲跌幅過濾，
        // 讓這個測試只驗證「複製是否完整」這件事。
        snapshot.last_close = Decimal::new(100, 0);
        snapshot.volume = Decimal::new(1234, 0);
        snapshots.insert(symbol.to_string(), snapshot);
    }
    share.set_stock_snapshots(snapshots);

    let mut symbols: Vec<String> = share
        .all_stock_snapshots()
        .into_iter()
        .map(|snapshot| snapshot.symbol)
        .collect();
    // HashMap 不保證順序，排序後再比對，避免測試因雜湊順序偶發失敗。
    symbols.sort();
    assert_eq!(symbols, vec!["2317", "2330", "6446"]);
}

#[test]
fn test_set_stock_snapshot_price_with_source_updates_source_site() {
    let share = Share::new();
    let mut snapshot = RealtimeSnapshot::new("2330".to_string(), Decimal::new(998, 0));
    snapshot.source_site = "Yahoo".to_string();

    let mut snapshots = HashMap::new();
    snapshots.insert("2330".to_string(), snapshot);
    share.set_stock_snapshots(snapshots);

    share.set_stock_snapshot_price_with_source("2330".to_string(), Decimal::new(1000, 0), "Fugle");

    let updated = share.get_stock_snapshot("2330").unwrap();
    assert_eq!(updated.price, Decimal::new(1000, 0));
    assert_eq!(updated.source_site, "Fugle");
}

#[test]
fn set_stock_snapshots_filters_invalid_new_prices_and_keeps_old_valid_snapshot() {
    let share = Share::new();
    let mut old_snapshot = RealtimeSnapshot::new("2330".to_string(), Decimal::new(100, 0));
    old_snapshot.last_close = Decimal::new(100, 0);
    old_snapshot.source_site = "old".to_string();
    let mut old_map = HashMap::new();
    old_map.insert("2330".to_string(), old_snapshot.clone());
    share.set_stock_snapshots(old_map);

    let mut invalid_update = RealtimeSnapshot::new("2330".to_string(), Decimal::new(200, 0));
    invalid_update.last_close = Decimal::new(100, 0);
    invalid_update.source_site = "new".to_string();
    let mut invalid_map = HashMap::new();
    invalid_map.insert("2330".to_string(), invalid_update);
    invalid_map.insert(
        "2317".to_string(),
        RealtimeSnapshot::new("2317".to_string(), Decimal::ZERO),
    );

    share.set_stock_snapshots(invalid_map);

    let kept = share.get_stock_snapshot("2330").unwrap();
    assert_eq!(kept.price, old_snapshot.price);
    assert_eq!(kept.source_site, "old");
    assert_eq!(share.get_stock_snapshot("2317"), None);
}

/// 在測試用 `Share` 的最後交易日報價快取寫入單筆收盤價。
fn seed_db_last_close(share: &Share, symbol: &str, closing_price: Decimal) {
    use crate::infra::database::table::last_daily_quotes::LastDailyQuotes;

    let mut quote = LastDailyQuotes::new();
    quote.stock_symbol = symbol.to_string();
    quote.closing_price = closing_price;
    share
        .last_trading_day_quotes
        .write()
        .unwrap()
        .insert(symbol.to_string(), quote);
}

/// 除息日：資料庫收盤價是除息前價格，站點昨收是除息參考價，成交價只貼近參考價也要放行。
#[test]
fn is_valid_price_accepts_price_near_site_reference_on_ex_dividend_day() {
    let share = Share::new();
    // 2542 興富發 2026-09-23 除息 4 元：除息前收盤 45.45、參考價 41.45
    seed_db_last_close(&share, "2542", dec!(45.45));

    assert!(share.is_valid_price("2542", dec!(39.05), dec!(41.45)));
    // 站點仍回報除息前收盤時，無法得知參考價，維持過濾
    assert!(!share.is_valid_price("2542", dec!(39.05), dec!(45.45)));
}

/// 1235 興泰 2026-09-24 除權息：資料庫與 HiStock 都給除權息前收盤 41.15，
/// 只有自行計算的參考價 38.71 能讓當天 35.5～40.2 的正常成交通過。
#[test]
fn is_valid_price_accepts_price_near_computed_ex_rights_reference() {
    let share = Share::new();
    seed_db_last_close(&share, "1235", dec!(41.15));
    assert!(!share.is_valid_price("1235", dec!(35.8), dec!(41.15)));

    share.set_ex_rights_reference_prices(HashMap::from([("1235".to_string(), dec!(38.71))]));
    assert!(share.is_valid_price("1235", dec!(35.8), dec!(41.15)));
    // 參考價的 ±10.5% 之外仍要過濾
    assert!(!share.is_valid_price("1235", dec!(30), dec!(41.15)));

    // 下一個交易日整批覆寫後，前一天的參考價不再生效
    share.set_ex_rights_reference_prices(HashMap::new());
    assert!(!share.is_valid_price("1235", dec!(35.8), dec!(41.15)));
}

/// 任一基準通過即有效，但兩個基準都差太多時仍要過濾。
#[test]
fn is_valid_price_rejects_price_far_from_both_baselines() {
    let share = Share::new();
    seed_db_last_close(&share, "6456", dec!(73.6));

    assert!(share.is_valid_price("6456", dec!(78.6), dec!(73.6)));
    assert!(!share.is_valid_price("6456", dec!(188), dec!(73.6)));
    assert!(!share.is_valid_price("6456", Decimal::ZERO, dec!(73.6)));
}

/// 沒有任何基準時無法比對，視為有效；只有站點基準時以站點基準比對。
#[test]
fn is_valid_price_falls_back_to_site_baseline_when_db_missing() {
    let share = Share::new();

    assert!(share.is_valid_price("2330", dec!(1000), Decimal::ZERO));
    assert!(share.is_valid_price("2330", dec!(1000), dec!(990)));
    assert!(!share.is_valid_price("2330", dec!(1200), dec!(990)));
}

/// 興櫃沒有漲跌幅限制：主檔標為興櫃時放寬到基準的 1/3～3 倍，但仍擋明顯錯誤的值。
#[test]
fn is_valid_price_relaxes_tolerance_for_emerging_stocks() {
    use crate::domain::registry::entity::Stock;

    let share = Share::new();
    // 4925 智微：上櫃規則下 +14% 會被濾掉
    assert!(!share.is_valid_price("4925", dec!(133.5), dec!(117.09)));

    share.stocks.write().unwrap().insert(
        "4925".to_string(),
        Stock::register("4925".to_string(), "智微".to_string(), 5, 1),
    );
    assert!(share.is_valid_price("4925", dec!(133.5), dec!(117.09)));
    assert!(!share.is_valid_price("4925", dec!(1.2), dec!(117.09)));
}

/// 比對「是不是這一檔」用的昨收：上市櫃回資料庫收盤價，興櫃（昨收是前一日加權均價）與沒有資料的回 None。
#[test]
fn comparable_last_close_skips_emerging_and_missing() {
    use crate::domain::registry::entity::Stock;

    let share = Share::new();
    seed_db_last_close(&share, "2330", dec!(1450));
    seed_db_last_close(&share, "2733", dec!(8.61));
    share.stocks.write().unwrap().insert(
        "2733".to_string(),
        Stock::register("2733".to_string(), "維格".to_string(), 5, 1),
    );

    assert_eq!(share.get_comparable_last_close("2330"), Some(dec!(1450)));
    assert_eq!(share.get_comparable_last_close("2733"), None);
    assert_eq!(share.get_comparable_last_close("79967"), None);
}

/// 2733 維格 2026-10-05：參考價 8.61、成交 14～16，是真實行情不是錯價；
/// 區間邊界（剛好 3 倍、1/3）算有效，超出才擋。
#[test]
fn emerging_band_accepts_large_real_moves_and_rejects_garbage() {
    use crate::domain::registry::entity::Stock;

    let share = Share::new();
    share.stocks.write().unwrap().insert(
        "2733".to_string(),
        Stock::register("2733".to_string(), "維格".to_string(), 5, 1),
    );

    assert!(share.is_valid_price("2733", dec!(14.75), dec!(8.61)));
    assert!(share.is_valid_price("2733", dec!(16.05), dec!(8.61)));
    assert!(share.is_valid_price("2733", dec!(25.83), dec!(8.61)));
    assert!(share.is_valid_price("2733", dec!(2.87), dec!(8.61)));
    assert!(!share.is_valid_price("2733", dec!(25.84), dec!(8.61)));
    assert!(!share.is_valid_price("2733", dec!(2.86), dec!(8.61)));
    // 興櫃類股（呼叫端指定市場別）走同一條規則。
    assert!(share.is_valid_price_for_market("79967", dec!(14.75), dec!(8.61), true));
    assert!(!share.is_valid_price_for_market("79967", dec!(0.08), dec!(227.5), true));
}

/// 當天有漲跌停價時照區間判斷：槓桿 ETF 的 ±20% 不被固定 10.5% 誤濾，超出區間仍擋下。
#[test]
fn todays_price_limit_range_overrides_the_fixed_tolerance() {
    use super::super::realtime::PriceLimit;

    let share = Share::new();
    let today = chrono::Local::now().date_naive();
    // 00631L：昨收 39.62，漲跌停 31.70～47.54。
    assert!(!share.is_valid_price("00631L", dec!(45), dec!(39.62)));
    share.set_price_limits(
        today,
        [(
            "00631L".to_string(),
            PriceLimit::Range {
                down: dec!(31.70),
                up: dec!(47.54),
            },
        )],
    );
    assert!(share.is_valid_price("00631L", dec!(45), dec!(39.62)));
    assert!(share.is_valid_price("00631L", dec!(47.54), dec!(39.62)));
    assert!(!share.is_valid_price("00631L", dec!(47.6), dec!(39.62)));
    assert!(!share.is_valid_price("00631L", dec!(31.6), dec!(39.62)));
}

/// 沒有漲跌幅限制的 ETF 改用寬鬆門檻（00715L 2026-10-02 上漲 11.3%），明顯錯誤的值仍擋下。
#[test]
fn unlimited_securities_use_the_relaxed_tolerance() {
    use super::super::realtime::PriceLimit;

    let share = Share::new();
    let today = chrono::Local::now().date_naive();
    assert!(!share.is_valid_price("00715L", dec!(70.75), dec!(63.55)));
    share.set_price_limits(today, [("00715L".to_string(), PriceLimit::Unlimited)]);
    assert!(share.is_valid_price("00715L", dec!(70.75), dec!(63.55)));
    assert!(!share.is_valid_price("00715L", dec!(0.5), dec!(63.55)));
}

/// 前一天的漲跌停價不適用今天；`Unknown` 不會被記錄。
#[test]
fn ipo_uses_todays_source_limits_and_resumes_normal_limits() {
    use super::super::realtime::PriceLimit;
    let share = Share::new();
    let today = chrono::Local::now().date_naive();
    // 普通股未建主檔也可依來源辨識無漲跌幅限制，不使用主檔匯入日期猜測 IPO。
    share.set_price_limits(today, [("7999".to_string(), PriceLimit::Unlimited)]);
    assert!(share.is_valid_price("7999", dec!(180), dec!(100)));
    assert!(!share.is_valid_price("7999", dec!(0.08), dec!(100)));
    share.set_price_limits(
        today,
        [(
            "7999".to_string(),
            PriceLimit::Range {
                down: dec!(90),
                up: dec!(110),
            },
        )],
    );
    assert!(!share.is_valid_price("7999", dec!(180), dec!(100)));
    assert!(share.is_valid_price("7999", dec!(110), dec!(100)));
}

/// 前一天的漲跌停價不適用今天；`Unknown` 不會被記錄。
#[test]
fn stale_or_unknown_price_limits_are_ignored() {
    use super::super::realtime::PriceLimit;

    let share = Share::new();
    let today = chrono::Local::now().date_naive();
    let yesterday = today - chrono::Duration::days(1);
    share.set_price_limits(
        yesterday,
        [(
            "00631L".to_string(),
            PriceLimit::Range {
                down: dec!(31.70),
                up: dec!(47.54),
            },
        )],
    );
    assert_eq!(share.price_limit_on("00631L", today), PriceLimit::Unknown);
    assert!(!share.is_valid_price("00631L", dec!(45), dec!(39.62)));

    share.set_price_limits(today, [("2330".to_string(), PriceLimit::Unknown)]);
    assert_eq!(share.price_limit_on("2330", today), PriceLimit::Unknown);
}

/// 來源已知是興櫃時（Yahoo 興櫃類股），主檔沒有該代號也適用興櫃閾值。
#[test]
fn is_valid_price_for_market_uses_caller_hint() {
    let share = Share::new();
    assert!(share.is_valid_price_for_market("7934", dec!(630), dec!(552.86), true));
    assert!(!share.is_valid_price_for_market("7934", dec!(630), dec!(552.86), false));
}

#[test]
fn set_stock_snapshot_price_rejects_outliers_and_accepts_valid_updates() {
    let share = Share::new();
    let mut snapshot = RealtimeSnapshot::new("2330".to_string(), Decimal::new(100, 0));
    snapshot.last_close = Decimal::new(100, 0);
    let mut snapshots = HashMap::new();
    snapshots.insert("2330".to_string(), snapshot);
    share.set_stock_snapshots(snapshots);

    share.set_stock_snapshot_price("2330".to_string(), Decimal::new(200, 0));
    assert_eq!(
        share.get_stock_snapshot("2330").unwrap().price,
        Decimal::new(100, 0)
    );

    share.set_stock_snapshot_price_with_source("2330".to_string(), Decimal::new(105, 0), "Yahoo");
    let updated = share.get_stock_snapshot("2330").unwrap();
    assert_eq!(updated.price, Decimal::new(105, 0));
    assert_eq!(updated.source_site, "Yahoo");
}
