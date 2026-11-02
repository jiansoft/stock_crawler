use super::*;

fn plain(symbol: &str, name: &str) -> Classification {
    classify(symbol, name, &ClassifyHint::default())
}

#[test]
fn suffix_decides_leveraged_inverse_commodity_and_bond() {
    assert_eq!(
        plain("00631L", "元大台灣50正2").strategy,
        Strategy::Leveraged
    );
    assert_eq!(plain("00632R", "元大台灣50反1").strategy, Strategy::Inverse);
    assert_eq!(
        plain("00635U", "期元大S&P黃金").strategy,
        Strategy::Commodity
    );
    assert_eq!(plain("00679B", "元大美債20年").strategy, Strategy::Bond);
    assert_eq!(plain("00981D", "主動中信非投等債").strategy, Strategy::Bond);
    assert_eq!(plain("00687C", "國泰20年美債+櫃U").strategy, Strategy::Bond);
}

#[test]
fn active_suffix_and_hint_win_over_high_dividend_words() {
    assert_eq!(
        plain("00406A", "主動中信台灣收益").strategy,
        Strategy::Active
    );
    assert_eq!(
        plain("00981T", "平衡凱基雙核收息").strategy,
        Strategy::Active
    );
    let hint = ClassifyHint {
        is_active: true,
        ..Default::default()
    };
    assert_eq!(
        classify("009999", "某某收益", &hint).strategy,
        Strategy::Active
    );
}

#[test]
fn names_decide_high_dividend_market_cap_overseas_and_thematic() {
    assert_eq!(plain("0056", "元大高股息").strategy, Strategy::HighDividend);
    assert_eq!(
        plain("00929", "復華台灣科技優息").strategy,
        Strategy::HighDividend
    );
    assert_eq!(plain("0050", "元大台灣50").strategy, Strategy::MarketCap);
    assert_eq!(plain("006208", "富邦台50").strategy, Strategy::MarketCap);
    assert_eq!(plain("00646", "元大S&P500").strategy, Strategy::Overseas);
    assert_eq!(plain("0052", "富邦科技").strategy, Strategy::Thematic);
}

#[test]
fn benchmark_hint_marks_market_cap() {
    let hint = ClassifyHint {
        benchmark: "臺灣50指數",
        ..Default::default()
    };
    assert_eq!(
        classify("009816", "凱基台灣TOP", &hint).strategy,
        Strategy::MarketCap
    );
    let msci = ClassifyHint {
        benchmark: "MSCI臺灣指數",
        ..Default::default()
    };
    assert_eq!(
        classify("0057", "富邦摩台", &msci).strategy,
        Strategy::MarketCap
    );
    assert_eq!(
        plain("00918", "大華優利高填息30").strategy,
        Strategy::HighDividend
    );
}

#[test]
fn region_prefers_hint_then_name_then_default() {
    let hint = ClassifyHint {
        region: "臺灣",
        ..Default::default()
    };
    assert_eq!(classify("0050", "元大台灣50", &hint).region, "台灣");
    assert_eq!(plain("00753L", "中信中國50正2").region, "中國");
    assert_eq!(plain("00657", "國泰日經225").region, "日本");
    assert_eq!(plain("00830", "國泰費城半導體").region, "美國");
    assert_eq!(plain("00720B", "元大投資級公司債").region, "");
    assert_eq!(plain("0056", "元大高股息").region, "台灣");
}

#[test]
fn themes_follow_keywords_and_foreign_currency_suffix() {
    assert_eq!(
        plain("00830", "國泰費城半導體").themes,
        vec!["半導體".to_string()]
    );
    assert_eq!(
        plain("00893", "國泰智能電動車").themes,
        vec!["人工智慧".to_string(), "電動車".to_string()]
    );
    assert_eq!(
        plain("00636K", "國泰中國A50+U").themes,
        vec!["外幣計價".to_string()]
    );
    assert!(plain("0056", "元大高股息").themes.is_empty());
    assert_eq!(Strategy::HighDividend.code(), "high_dividend");
}

#[test]
fn all_strategy_codes_are_distinct() {
    let all = [
        Strategy::MarketCap,
        Strategy::HighDividend,
        Strategy::Active,
        Strategy::Bond,
        Strategy::Leveraged,
        Strategy::Inverse,
        Strategy::Commodity,
        Strategy::Overseas,
        Strategy::Thematic,
    ];
    let codes: std::collections::HashSet<_> = all.iter().map(|s| s.code()).collect();
    assert_eq!(codes.len(), all.len());
}

#[test]
fn dividend_frequency_maps_source_values() {
    assert_eq!(dividend_frequency("Monthly"), "monthly");
    assert_eq!(dividend_frequency("Quarterly"), "quarterly");
    assert_eq!(dividend_frequency("Semi-Annual"), "semi_annual");
    assert_eq!(dividend_frequency("Annual"), "annual");
    assert_eq!(dividend_frequency(""), "");
    assert_eq!(dividend_frequency("Irregular"), "");
}
