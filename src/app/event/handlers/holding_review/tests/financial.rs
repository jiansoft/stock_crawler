//! 持股財報通知（EPS、三率、去年同期比較）的訊息測試。

use super::*;

// 三率都比去年同季高時列出各自的百分點變化並標三率三升；都下降標三率三降。
#[test]
fn financial_message_shows_margin_changes_and_trend() {
    let msg = EventDispatcher::build_financial_message(
        2026,
        "Q2",
        &[financial_alert(Some(dec!(7.01)))],
        None,
    );
    assert!(
        msg.contains(
            r"毛利率︰53\.1%（\+2\.3） 營益率︰42\.5%（\+2\.5） 淨利率︰38\.2%（\+1\.5） 📈三率三升"
        ),
        "{msg}"
    );

    let mut down = financial_alert(Some(dec!(7.01)));
    down.last_year_gross_profit = Some(dec!(60));
    down.last_year_operating_margin = Some(dec!(50));
    down.last_year_net_margin = Some(dec!(40));
    assert_eq!(down.margin_trend(), Some(MarginTrend::AllDown));
    let msg = EventDispatcher::build_financial_message(2026, "Q2", &[down], None);
    assert!(msg.contains(r"（\-6\.9）"), "{msg}");
    assert!(msg.contains("📉三率三降"), "{msg}");

    let mut mixed = financial_alert(Some(dec!(7.01)));
    mixed.last_year_net_margin = Some(dec!(40));
    assert_eq!(mixed.margin_trend(), Some(MarginTrend::Mixed));
    let msg = EventDispatcher::build_financial_message(2026, "Q2", &[mixed], None);
    assert!(!msg.contains("三率"), "{msg}");
}

// 營益率、淨利率為 0（來源沒有提供）時不列也不比較；去年同季為 0 時不附變化、不判斷三率。
#[test]
fn financial_message_skips_missing_margins() {
    let mut alert = financial_alert(Some(dec!(7.01)));
    alert.operating_margin = Decimal::ZERO;
    alert.last_year_net_margin = Some(Decimal::ZERO);
    assert_eq!(alert.margin_changes(), [Some(dec!(2.3)), None, None]);
    assert_eq!(alert.margin_trend(), None);
    let msg = EventDispatcher::build_financial_message(2026, "Q2", &[alert], None);
    assert!(!msg.contains("營益率"), "{msg}");
    assert!(msg.contains(r"淨利率︰38\.2%"), "{msg}");
    assert!(!msg.contains(r"38\.2%（"), "{msg}");
    assert!(!msg.contains("三率"), "{msg}");

    let msg = EventDispatcher::build_financial_message(2026, "Q2", &[financial_alert(None)], None);
    assert!(
        msg.contains(r"毛利率︰53\.1% 營益率︰42\.5% 淨利率︰38\.2%"),
        "{msg}"
    );
}

#[test]
fn financial_message_includes_year_over_year_comparison() {
    let msg = EventDispatcher::build_financial_message(
        2026,
        "Q2",
        &[financial_alert(Some(dec!(7.01)))],
        None,
    );

    assert!(msg.contains("去年同季"), "{msg}");
    assert!(msg.contains("7\\.01"), "{msg}");
    assert!(msg.contains("\\+2\\.55"), "差額要帶正號並跳脫：{msg}");
}

// 查不到去年同季時不能顯示 0，那看起來像獲利歸零。
#[test]
fn financial_message_omits_comparison_when_last_year_is_missing() {
    let msg = EventDispatcher::build_financial_message(2026, "Q2", &[financial_alert(None)], None);

    assert!(!msg.contains("去年同季"), "{msg}");
    assert!(msg.contains("9\\.56"), "{msg}");
}

#[test]
fn financial_message_marks_new_holdings_on_follow_up() {
    let new_symbols = BTreeSet::from(["2330".to_string()]);
    let msg = EventDispatcher::build_financial_message(
        2026,
        "Q2",
        &[financial_alert(None)],
        Some(&new_symbols),
    );

    assert!(msg.contains("持股財報（新公布 1 檔）︰"), "{msg}");
    assert!(msg.contains("🆕 [2330]"), "{msg}");
}
