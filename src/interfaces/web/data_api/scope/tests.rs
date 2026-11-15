//! 持股類端點金鑰與資料範圍判斷的單元測試（不需資料庫）。

use super::*;

/// 開關開啟：只收個人金鑰；關閉：市場金鑰也收但要寫日誌；錯的金鑰一律拒絕。
#[test]
fn key_decision_follows_strict_switch() {
    assert_eq!(
        key_decision(SuppliedKey::Personal, true),
        KeyDecision::Allow
    );
    assert_eq!(
        key_decision(SuppliedKey::Personal, false),
        KeyDecision::Allow
    );
    assert_eq!(key_decision(SuppliedKey::Market, true), KeyDecision::Reject);
    assert_eq!(
        key_decision(SuppliedKey::Market, false),
        KeyDecision::AllowLegacy
    );
    assert_eq!(
        key_decision(SuppliedKey::Invalid, true),
        KeyDecision::Reject
    );
    assert_eq!(
        key_decision(SuppliedKey::Invalid, false),
        KeyDecision::Reject
    );
}

/// 開關開啟時缺 household_id 回 400；關閉時視為家庭 #1 並寫日誌（不檢查會員，與舊版相同）。
#[test]
fn scope_decision_handles_missing_household() {
    assert_eq!(
        scope_decision(None, Some(2), true),
        ScopeDecision::MissingHousehold
    );
    assert_eq!(
        scope_decision(None, None, true),
        ScopeDecision::MissingHousehold
    );
    assert_eq!(
        scope_decision(None, Some(2), false),
        ScopeDecision::AllowLegacy
    );
    assert_eq!(
        scope_decision(None, None, false),
        ScopeDecision::AllowLegacy
    );
}

/// 帶了 household_id：不論開關都照新規則——指定會員要查歸屬，0 或不帶為家庭合計。
#[test]
fn scope_decision_with_household_verifies_member() {
    for strict in [true, false] {
        assert_eq!(scope_decision(Some(1), None, strict), ScopeDecision::Allow);
        assert_eq!(
            scope_decision(Some(1), Some(0), strict),
            ScopeDecision::Allow
        );
        assert_eq!(
            scope_decision(Some(1), Some(3), strict),
            ScopeDecision::VerifyMember {
                household_id: 1,
                member_id: 3,
            }
        );
        assert!(matches!(
            scope_decision(Some(0), Some(3), strict),
            ScopeDecision::Invalid(_)
        ));
        assert!(matches!(
            scope_decision(Some(1), Some(-1), strict),
            ScopeDecision::Invalid(_)
        ));
    }
}

/// 啟動檢查：註冊模式不是 closed 時開關必須開啟；未設定視為 closed。
#[test]
fn startup_requires_strict_when_registration_is_open() {
    assert!(validate_startup(None, false).is_ok());
    assert!(validate_startup(Some("closed"), false).is_ok());
    assert!(validate_startup(Some(" Closed "), false).is_ok());
    assert!(validate_startup(Some(""), false).is_ok());
    assert!(validate_startup(Some("invite"), false).is_err());
    assert!(validate_startup(Some("open"), false).is_err());
    assert!(validate_startup(Some("invite"), true).is_ok());
    assert!(validate_startup(Some("open"), true).is_ok());
}
