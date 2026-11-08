//! 具名策略：回測各端點的 `preset=名稱` 參數，以及 `/strategy-presets` 清單端點的測試。

use super::*;

/// `preset` 的值會解碼（含 `+` 空白與中文）並去掉前後空白；沒有或空白為 None。
#[test]
fn preset_name_decodes_value() {
    assert_eq!(
        preset_name("limit=5&preset=%E6%9C%80%E4%BD%B3+A"),
        Some("最佳 A".to_owned())
    );
    assert_eq!(preset_name("preset=%20"), None);
    assert_eq!(preset_name("limit=5&&presets=x"), None);
    assert_eq!(preset_name(""), None);
}

/// 請求的參數覆蓋同名參數、其餘接在後面；`preset` 不保留（含存的 query 裡誤存的）。
#[test]
fn merge_query_lets_request_override() {
    assert_eq!(
        merge_query(
            "limit=20&hold_days=20&preset=x&from=2019-01-02",
            "preset=a&hold_days=60&rebalance_offset=3"
        ),
        "limit=20&from=2019-01-02&hold_days=60&rebalance_offset=3"
    );
    assert_eq!(merge_query("limit=20", "preset=a"), "limit=20");
    assert_eq!(merge_query("", "preset=a&flag"), "flag=");
}

/// 解析失敗回錯誤訊息；成功時與一般 query 相同。
#[test]
fn parse_params_reports_errors() {
    let params = parse_params("limit=5&from=2019-01-02").expect("可解析");
    assert_eq!(params.limit, Some(5));
    assert_eq!(params.from.as_deref(), Some("2019-01-02"));
    assert!(parse_params("limit=abc").is_err());
    assert!(parse_params("limit=5 6").is_err());
}
