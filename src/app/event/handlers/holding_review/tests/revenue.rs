//! 持股月營收通知（年增、推估 EPS、創新高與新持股標示）的訊息測試。

use super::*;

#[test]
fn revenue_message_lists_every_alert_with_escaped_values() {
    let msg = EventDispatcher::build_revenue_message(
        202608,
        &[
            revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1)),
            revenue_alert("2454", "聯發科", dec!(-25.5), dec!(1.2)),
        ],
        None,
    );

    assert!(msg.contains("2026\\-08"), "月份的連字號要跳脫：{msg}");
    assert!(msg.contains("台積電"), "{msg}");
    assert!(msg.contains("聯發科"), "{msg}");
    // 小數點是 MarkdownV2 保留字元，沒跳脫整則訊息會被 Bot API 退回。
    assert!(msg.contains("\\+33\\.25"), "{msg}");
    assert!(msg.contains("\\-25\\.5"), "{msg}");
}

#[test]
fn revenue_message_puts_industry_before_stock_name() {
    let msg = EventDispatcher::build_revenue_message(
        202608,
        &[revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1))],
        None,
    );

    assert!(msg.contains("半導體業 台積電"), "產業別要接在股名前：{msg}");
}

// 查無產業分類時整段省略，不留下多餘空白或空字串。
#[test]
fn revenue_message_omits_industry_when_unknown() {
    let mut alert = revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1));
    alert.industry_name = None;

    let msg = EventDispatcher::build_revenue_message(202608, &[alert], None);

    assert!(msg.contains(") 台積電 營收︰"), "{msg}");
}

// 錨點 EPS 3 元 × (累計 1,000,000 ÷ 錨點 600,000) ＝ 5 元；8 個月年化 ＝ 7.5 元。
#[test]
fn revenue_message_shows_estimated_eps_from_accumulated_revenue() {
    let msg = EventDispatcher::build_revenue_message(
        202608,
        &[revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1))],
        None,
    );

    assert!(msg.contains("累計︰1,000,000"), "{msg}");
    assert!(msg.contains(r"累計年增︰\+12\.5%"), "{msg}");
    assert!(msg.contains(r"推估EPS︰5（年估 7\.5）"), "{msg}");
}

// 今年還沒公布季報時退回淨利率法，標籤要換成「概估」以示區別。
#[test]
fn revenue_message_labels_margin_fallback_as_rough_estimate() {
    let mut alert = revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1));
    alert.anchor_eps = None;
    alert.anchor_accumulated_revenue = None;

    let msg = EventDispatcher::build_revenue_message(202608, &[alert], None);

    assert!(msg.contains("概估EPS︰4"), "{msg}");
    assert!(!msg.contains("推估EPS"), "{msg}");
}

// 兩種推估法都缺料時整段省略，不要把「查無資料」畫成 0。
#[test]
fn revenue_message_omits_estimated_eps_when_data_is_missing() {
    let mut alert = revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1));
    alert.anchor_eps = None;
    alert.anchor_accumulated_revenue = None;
    alert.net_income_margin = None;

    let msg = EventDispatcher::build_revenue_message(202608, &[alert], None);

    assert!(!msg.contains("估EPS"), "{msg}");
}

// 營收劇變時在推估值後面標註只供參考。
#[test]
fn revenue_message_flags_volatile_estimate() {
    let mut alert = revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1));
    alert.monthly_accumulated = dec!(1200000);

    let msg = EventDispatcher::build_revenue_message(202608, &[alert], None);

    assert!(msg.contains("⚠️營收劇變僅供參考"), "{msg}");
}

// 12 月的累計就是全年，年估與累計相同時不再重複列出。
#[test]
fn revenue_message_omits_annual_estimate_in_december() {
    let mut alert = revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1));
    alert.date = 202612;

    let msg = EventDispatcher::build_revenue_message(202612, &[alert], None);

    assert!(msg.contains("推估EPS︰5"), "{msg}");
    assert!(!msg.contains("年估"), "{msg}");
}

// 排序由 SQL 決定，訊息必須原封不動地照傳入順序輸出。
#[test]
fn revenue_message_keeps_input_order() {
    let msg = EventDispatcher::build_revenue_message(
        202608,
        &[
            revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1)),
            revenue_alert("2454", "聯發科", dec!(-25.5), dec!(1.2)),
        ],
        None,
    );

    let tsmc = msg.find("台積電").expect("台積電 應該在訊息中");
    let mtk = msg.find("聯發科").expect("聯發科 應該在訊息中");
    assert!(tsmc < mtk, "{msg}");
}

// 標題要標明是全部持股與排序方式，不再出現舊版的年增率門檻。
#[test]
fn revenue_message_header_reports_count_and_sorting() {
    let msg = EventDispatcher::build_revenue_message(
        202608,
        &[
            revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1)),
            revenue_alert("2454", "聯發科", dec!(-25.5), dec!(1.2)),
        ],
        None,
    );

    assert!(msg.contains("共 2 檔，依年增率排序"), "{msg}");
}

// 補發時只標出新增的持股，標題補上新增檔數。
#[test]
fn revenue_message_marks_new_holdings_on_follow_up() {
    let new_symbols = BTreeSet::from(["2454".to_string()]);
    let msg = EventDispatcher::build_revenue_message(
        202608,
        &[
            revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1)),
            revenue_alert("2454", "聯發科", dec!(-25.5), dec!(1.2)),
        ],
        Some(&new_symbols),
    );

    assert!(msg.contains("共 2 檔，新公布 1 檔"), "{msg}");
    assert!(msg.contains("🆕 [2454]"), "{msg}");
    assert!(!msg.contains("🆕 [2330]"), "{msg}");
}

// 首次通知全部都是新的，不需要逐檔標記。
#[test]
fn revenue_message_has_no_marks_on_first_notification() {
    let msg = EventDispatcher::build_revenue_message(
        202608,
        &[revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1))],
        None,
    );

    assert!(!msg.contains('🆕'), "{msg}");
    assert!(!msg.contains("新公布"), "{msg}");
}

// 月營收創歷史新高標🏆；之前不滿 12 個月（previous_high 為 None）不判斷；連續年減滿 3 個月標📉，
// 創新高優先。
#[test]
fn revenue_message_marks_record_highs_and_declines() {
    let mut record = revenue_alert("2330", "台積電", dec!(30), dec!(5));
    record.previous_high = Some(dec!(200000000));
    assert!(record.is_record_high());
    let mut tied = revenue_alert("2317", "鴻海", dec!(10), dec!(1));
    tied.previous_high = Some(dec!(250000000));
    assert!(!tied.is_record_high(), "追平不算創新高");
    let mut declining = revenue_alert("1101", "台泥", dec!(-8), dec!(-1));
    declining.yoy_decline_months = 3;
    let mut short = revenue_alert("1102", "亞泥", dec!(-3), dec!(-1));
    short.yoy_decline_months = 2;

    let msg =
        EventDispatcher::build_revenue_message(202608, &[record, tied, declining, short], None);
    let line = |symbol: &str| {
        msg.lines()
            .find(|line| line.contains(symbol) && line.contains("營收︰"))
            .unwrap_or_default()
            .to_owned()
    };
    assert!(line("2330").ends_with("🏆創歷史新高"), "{msg}");
    assert!(!line("2317").contains("🏆"), "{msg}");
    assert!(line("1101").ends_with("📉連續 3 月年減"), "{msg}");
    assert!(!line("1102").contains("📉"), "{msg}");
}
