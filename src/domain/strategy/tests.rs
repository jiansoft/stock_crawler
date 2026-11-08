//! 具名策略的測試。

use super::*;

/// 名稱去掉前後空白、長度 1–64 字元（以字元計，中文也算一個），不可含 query string 保留字元。
#[test]
fn normalize_name_trims_and_rejects_reserved() {
    assert_eq!(normalize_name("  最佳組合 "), Ok("最佳組合".to_owned()));
    assert_eq!(
        normalize_name(&"甲".repeat(64)).map(|name| name.chars().count()),
        Ok(64)
    );
    assert!(normalize_name(&"甲".repeat(65)).is_err());
    assert!(normalize_name("   ").is_err());
    for name in ["a&b", "a=b", "a#b", "a?b", "a\nb"] {
        assert!(normalize_name(name).is_err(), "{name}");
    }
}
