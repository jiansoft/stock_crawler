//! 通知項目目錄的測試。

use std::collections::HashSet;

use super::*;

/// 代碼不重複，前綴與受眾一致，格式符合 `notify_topic.code`（小寫、底線、一個點）。
#[test]
fn codes_are_unique_and_match_audience_prefix() {
    let mut seen = HashSet::new();
    for topic in Topic::ALL {
        let spec = topic.spec();
        assert!(seen.insert(spec.code), "代碼重複：{}", spec.code);
        let (prefix, rest) = spec.code.split_once('.').expect("代碼必須是「分類.名稱」");
        assert_eq!(
            prefix,
            spec.audience.as_str(),
            "{} 的前綴應與受眾一致",
            spec.code
        );
        assert!(
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{} 只能是小寫英文與底線",
            spec.code
        );
        assert!(
            spec.code.len() <= 64,
            "{} 超過 notify_topic.code 的長度",
            spec.code
        );
        assert!(
            !spec.name.is_empty() && !spec.description.is_empty(),
            "{} 缺名稱或說明",
            spec.code
        );
    }
    assert_eq!(seen.len(), Topic::ALL.len());
}

/// 受眾與需要的權限對應 7.3：持股看 portfolio.view（市值變化例外，看 money_history.view）、市場看 market.view、
/// 系統看 ops.view、價格追蹤看 price_trace.view。
#[test]
fn required_permission_follows_audience() {
    for topic in Topic::ALL {
        let spec = topic.spec();
        let expected = match spec.audience {
            Audience::Holdings if topic == Topic::HoldingsMarketValue => "money_history.view",
            Audience::Holdings => "portfolio.view",
            Audience::Market => "market.view",
            Audience::System => "ops.view",
            Audience::Personal => "price_trace.view",
        };
        assert_eq!(
            spec.required_permission, expected,
            "{} 的權限與受眾不符",
            spec.code
        );
    }
}

/// 只有 system.critical 限 root；市場類預設都不訂閱；系統類預設都訂閱（7.3 預設欄）。
#[test]
fn defaults_match_catalog() {
    let root_only: Vec<&str> = Topic::ALL
        .iter()
        .filter(|topic| topic.spec().root_only)
        .map(|topic| topic.code())
        .collect();
    assert_eq!(root_only, vec!["system.critical"]);
    for topic in Topic::ALL {
        let spec = topic.spec();
        match spec.audience {
            Audience::Market => assert!(!spec.default_enabled, "{} 市場類預設不訂閱", spec.code),
            Audience::System => assert!(spec.default_enabled, "{} 系統類預設訂閱", spec.code),
            Audience::Holdings | Audience::Personal => {}
        }
    }
    let enabled_holdings: Vec<&str> = Topic::ALL
        .iter()
        .filter(|topic| topic.audience() == Audience::Holdings && topic.spec().default_enabled)
        .map(|topic| topic.code())
        .collect();
    assert_eq!(
        enabled_holdings,
        vec![
            "holdings.ex_dividend_preview",
            "holdings.ex_dividend",
            "holdings.payable",
            "holdings.monthly_payout",
            "holdings.market_value",
            "holdings.revenue",
            "holdings.financial_report",
            "holdings.earnings_call",
            "holdings.financial_health",
            "holdings.technical",
        ]
    );
}

/// 分組只用 notify_topic.category 註解列出的值。
#[test]
fn categories_use_known_values() {
    let allowed = [
        "dividend",
        "value",
        "chip",
        "fundamental",
        "technical",
        "market",
        "price_trace",
        "system",
    ];
    for topic in Topic::ALL {
        assert!(
            allowed.contains(&topic.spec().category.as_str()),
            "{} 的分組不在清單內",
            topic.code()
        );
    }
}
