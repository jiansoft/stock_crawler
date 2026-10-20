//! 要通知的策略：具名策略與 .env 設定合併。

use super::*;

fn preset(name: &str, query: &str, alert: bool) -> StrategyPreset {
    StrategyPreset {
        name: name.to_owned(),
        query: query.to_owned(),
        description: String::new(),
        alert,
    }
}

/// 只取 `alert` 的具名策略；環境變數的條件空白或與具名策略相同時不重複加入。
#[test]
fn alert_strategies_merge_presets_and_env() {
    let presets = vec![
        preset("甲", " from=2019-01-02&limit=20 ", true),
        preset("乙", "from=2020-01-02", false),
    ];
    let strategies = alert_strategies(
        presets.clone(),
        Some("from=2021-01-04".to_owned()),
        Some(" 環境 ".to_owned()),
    );
    assert_eq!(
        strategies,
        [
            AlertStrategy {
                name: "甲".to_owned(),
                query: "from=2019-01-02&limit=20".to_owned()
            },
            AlertStrategy {
                name: "環境".to_owned(),
                query: "from=2021-01-04".to_owned()
            },
        ]
    );
    let same = alert_strategies(
        presets.clone(),
        Some("from=2019-01-02&limit=20".to_owned()),
        None,
    );
    assert_eq!(same.len(), 1);
    assert_eq!(
        alert_strategies(presets, Some("  ".to_owned()), None).len(),
        1
    );
    assert!(alert_strategies(Vec::new(), None, None).is_empty());
}
