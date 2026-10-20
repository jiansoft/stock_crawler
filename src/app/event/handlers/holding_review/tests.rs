use rust_decimal_macros::dec;

use super::*;

fn revenue_alert(symbol: &str, name: &str, yoy: Decimal, mom: Decimal) -> HoldingRevenueAlert {
    HoldingRevenueAlert {
        stock_symbol: symbol.to_string(),
        stock_name: name.to_string(),
        industry_name: Some("半導體業".to_string()),
        monthly: dec!(250000000),
        monthly_accumulated: dec!(1000000),
        compared_with_last_month: mom,
        compared_with_last_year_same_month: yoy,
        accumulated_compared_with_last_year: dec!(12.5),
        issued_share: 100_000_000,
        net_income_margin: Some(dec!(40)),
        // 錨點 EPS 3 元、錨點營收 600,000 千元 ⇒ 3 × (1,000,000 ÷ 600,000) ＝ 5 元。
        anchor_eps: Some(dec!(3)),
        anchor_accumulated_revenue: Some(dec!(600000)),
        anchor_quarter_no: Some(2),
        ttm_eps: None,
        ttm_revenue: None,
        reg_slope: None,
        reg_intercept: None,
        reg_r2: None,
        reg_quarters: None,
        previous_high: None,
        yoy_decline_months: 0,
        date: 202608,
    }
}

fn foreign_alert(
    symbol: &str,
    name: &str,
    change_20d: Option<Decimal>,
    streak: i32,
) -> HoldingForeignHoldingAlert {
    HoldingForeignHoldingAlert {
        stock_symbol: symbol.to_string(),
        stock_name: name.to_string(),
        date: NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
        share_holding_percentage: dec!(26.3700),
        change_5d: Some(dec!(1.0300)),
        change_20d,
        streak,
    }
}

#[test]
fn foreign_holding_message_groups_by_direction_and_sorts_by_magnitude() {
    let big_up = foreign_alert("6488", "環球晶", Some(dec!(5.2)), 4);
    let small_up = foreign_alert("3105", "穩懋", Some(dec!(3.1)), 1);
    let down = foreign_alert("8069", "元太", Some(dec!(-3.5)), -3);
    let date = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();

    let msg = EventDispatcher::build_foreign_holding_message(date, &[&small_up, &big_up], &[&down]);

    let increase_at = msg.find("外資增持").unwrap();
    let decrease_at = msg.find("外資減持").unwrap();
    assert!(increase_at < decrease_at);
    // 增持依 20 日變化由大到小：環球晶在穩懋前面
    assert!(msg.find("環球晶").unwrap() < msg.find("穩懋").unwrap());
    assert!(msg.contains("20日︰\\+5\\.2"));
    assert!(msg.contains("持股︰26\\.37%"));
    assert!(msg.contains("連續增持 4 日"));
    assert!(msg.contains("連續減持 3 日"));
    // 連續 1 日不註明
    assert_eq!(msg.matches("連續增持").count(), 1);
}

#[test]
fn foreign_holding_message_omits_empty_group() {
    let down = foreign_alert("8069", "元太", Some(dec!(-3.5)), -1);
    let msg = EventDispatcher::build_foreign_holding_message(
        NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
        &[],
        &[&down],
    );

    assert!(!msg.contains("外資增持"));
    assert!(msg.contains("外資減持"));
}

#[test]
fn format_optional_change_shows_dash_without_history() {
    assert_eq!(EventDispatcher::format_optional_change(None), "—");
    assert_eq!(
        EventDispatcher::format_optional_change(Some(dec!(1.2))),
        "+1.2"
    );
    assert_eq!(
        EventDispatcher::format_optional_change(Some(dec!(-0.5))),
        "-0.5"
    );
}

#[test]
fn foreign_holding_key_separates_direction_and_symbol() {
    assert_eq!(
        EventDispatcher::foreign_holding_key(ForeignHoldingDirection::Increase, "2330"),
        "holding_review:qfii:up:2330"
    );
    assert_eq!(
        EventDispatcher::foreign_holding_key(ForeignHoldingDirection::Decrease, "2330"),
        "holding_review:qfii:down:2330"
    );
}

fn financial_alert(last_year_eps: Option<Decimal>) -> HoldingFinancialAlert {
    HoldingFinancialAlert {
        stock_symbol: "2330".to_string(),
        stock_name: "台積電".to_string(),
        quarter: "Q2".to_string(),
        earnings_per_share: dec!(9.56),
        return_on_equity: dec!(8.4),
        gross_profit: dec!(53.1),
        operating_margin: dec!(42.5),
        net_margin: dec!(38.2),
        last_year_earnings_per_share: last_year_eps,
        last_year_gross_profit: last_year_eps.map(|_| dec!(50.8)),
        last_year_operating_margin: last_year_eps.map(|_| dec!(40)),
        last_year_net_margin: last_year_eps.map(|_| dec!(36.7)),
        year: 2026,
    }
}

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
fn format_revenue_month_pads_single_digit_month() {
    assert_eq!(EventDispatcher::format_revenue_month(202608), "2026-08");
    assert_eq!(EventDispatcher::format_revenue_month(202612), "2026-12");
}

#[test]
fn with_sign_only_prefixes_positive_values() {
    assert_eq!(EventDispatcher::with_sign(dec!(33.25)), "+33.25");
    assert_eq!(EventDispatcher::with_sign(dec!(-4.1)), "-4.1");
    assert_eq!(EventDispatcher::with_sign(Decimal::ZERO), "0");
}

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
fn parse_notified_symbols_ignores_blank_entries() {
    let symbols = EventDispatcher::parse_notified_symbols("2330, 2454,,");

    assert_eq!(
        symbols,
        BTreeSet::from(["2330".to_string(), "2454".to_string()])
    );
    assert!(EventDispatcher::parse_notified_symbols("").is_empty());
}

#[test]
fn find_new_symbols_excludes_already_notified() {
    let notified = BTreeSet::from(["2330".to_string()]);

    let new_symbols = EventDispatcher::find_new_symbols(&notified, ["2330", "2454"]);

    assert_eq!(new_symbols, BTreeSet::from(["2454".to_string()]));
    assert!(EventDispatcher::find_new_symbols(&notified, ["2330"]).is_empty());
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

#[test]
fn stock_link_escapes_dots_in_url() {
    let link = EventDispatcher::stock_link("2330");

    assert_eq!(link, "[2330](https://tw\\.stock\\.yahoo\\.com/quote/2330)");
}

/// 已通知集合存在 Redis：第一次全部是新的、不逐檔標記；之後只標記新增代號；沒有新增就不送出。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn send_if_new_holdings_tracks_notified_symbols_in_redis() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 send_if_new_holdings_tracks_notified_symbols_in_redis：無 Redis 連接");
        return;
    }
    let key = "test:holding_review:notified:79979";
    let _ = CLIENT.delete(key).await;

    let mut highlights = Vec::new();
    let first = EventDispatcher::send_if_new_holdings(key, 60, ["79979A", "79979B"], |new| {
        highlights.push(new.cloned());
        "第一次".to_owned()
    })
    .await;
    let repeated = EventDispatcher::send_if_new_holdings(key, 60, ["79979B", "79979A"], |new| {
        highlights.push(new.cloned());
        "重複".to_owned()
    })
    .await;
    let added = EventDispatcher::send_if_new_holdings(key, 60, ["79979A", "79979C"], |new| {
        highlights.push(new.cloned());
        "補發".to_owned()
    })
    .await;
    let stored = EventDispatcher::load_notified_symbols(key).await;
    let _ = CLIENT.delete(key).await;

    assert!(first && !repeated && added);
    assert_eq!(
        highlights,
        [None, Some(BTreeSet::from(["79979C".to_owned()]))]
    );
    assert_eq!(
        stored,
        BTreeSet::from([
            "79979A".to_owned(),
            "79979B".to_owned(),
            "79979C".to_owned()
        ])
    );
}

/// 外資持股去重鍵：寫入後視為已通知，刪除後恢復為未通知。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn is_already_notified_reads_redis_key() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 is_already_notified_reads_redis_key：無 Redis 連接");
        return;
    }
    let key = "test:holding_review:qfii:79979";
    let _ = CLIENT.delete(key).await;
    assert!(!EventDispatcher::is_already_notified(key).await);
    CLIENT.set(key, "2026-10-13", 60).await.expect("寫入去重鍵");
    assert!(EventDispatcher::is_already_notified(key).await);
    let _ = CLIENT.delete(key).await;
    assert!(!EventDispatcher::is_already_notified(key).await);
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
