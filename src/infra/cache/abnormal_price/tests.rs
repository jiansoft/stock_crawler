//! 異常價格過濾的紀錄與收盤統計的測試。

use rust_decimal_macros::dec;

use super::*;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("日期合法")
}

fn blocked(price: Decimal, last_close: Decimal) -> BlockedPrice {
    BlockedPrice { price, last_close }
}

/// 測試不在意價格時用的固定值。
fn any_price() -> BlockedPrice {
    blocked(Decimal::ONE, Decimal::TEN)
}

/// 每站當天只有第一筆算「第一筆」；不同站各自計；換日重新計。
#[test]
fn record_flags_first_per_site_per_day() {
    let mut stats = AbnormalPriceStats::default();
    assert!(stats.record(day(30), "HiStock", "3189", any_price()));
    assert!(!stats.record(day(30), "HiStock", "3481", any_price()));
    assert!(!stats.record(day(30), "HiStock", "3189", any_price()));
    assert!(stats.record(day(30), "CnYes", "2330", any_price()));
    assert!(
        stats.record(day(31), "HiStock", "6134", any_price()),
        "換日要重新算第一筆"
    );
    assert_eq!(stats.sites.len(), 1, "換日清掉前一天的統計");
}

/// 收盤行依站點列次數、檔數與代號，每檔附第一次被擋的採集價與昨收（去掉尾端的 0）；
/// 同檔重複只算一檔；超過上限的代號以省略號帶過。
#[test]
fn summary_lists_counts_per_site() {
    let mut stats = AbnormalPriceStats::default();
    assert_eq!(stats.summary(), None);
    stats.record(
        day(30),
        "HiStock",
        "3189",
        blocked(dec!(452.96), dec!(900.00)),
    );
    stats.record(day(30), "HiStock", "3481", blocked(dec!(1.59), dec!(42.5)));
    stats.record(day(30), "HiStock", "3189", blocked(dec!(1.00), dec!(900)));
    stats.record(day(30), "HiStock", "6134", blocked(dec!(30), dec!(20.10)));
    stats.record(day(30), UNKNOWN_SITE, "2330", blocked(dec!(0), dec!(1000)));
    assert_eq!(
        stats.summary().as_deref(),
        Some(concat!(
            "今日過濾異常價格：HiStock 4 次（3 檔：3189 452.96／昨收 900、3481 1.59／昨收 42.5、",
            "6134 30／昨收 20.1）；未標示來源 1 次（1 檔：2330 0／昨收 1000）"
        )),
        "3189 第二次被擋的 1.00 不覆蓋第一次的 452.96"
    );

    let mut many = AbnormalPriceStats::default();
    for n in 0..12 {
        many.record(day(30), "HiStock", &format!("10{n:02}"), any_price());
    }
    let line = many.summary().expect("有資料");
    assert!(line.contains("12 次（12 檔：1000 1／昨收 10、"), "{line}");
    assert!(line.ends_with("1009 1／昨收 10…）"), "{line}");
}

/// 對外入口：記錄後收盤輸出並清空，再輸出一次不會重複。
/// 用專屬站點名稱，同時執行的其他測試記到別的站點也不影響判斷。
#[test]
fn report_and_flush_clear_the_day() {
    const SITE: &str = "H20 測試站";
    report_abnormal_price("9999", Decimal::ONE, Decimal::TEN, SITE);
    report_abnormal_price("9999", Decimal::ONE, Decimal::TEN, SITE);
    let count = |site: &str| {
        STATS
            .lock()
            .expect("測試不會讓鎖中毒")
            .sites
            .get(site)
            .map_or(0, |count| count.count)
    };
    assert_eq!(count(SITE), 2);
    flush_abnormal_price_stats();
    assert_eq!(count(SITE), 0, "收盤輸出後清空");
    flush_abnormal_price_stats();
}

/// 沒有標示來源時歸到「未標示來源」，有來源照原名。
#[test]
fn empty_site_is_labelled_unknown() {
    assert_eq!(site_label(""), UNKNOWN_SITE);
    assert_eq!(site_label("HiStock"), "HiStock");
}
