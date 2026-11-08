//! `/market/dividend-yield-ranking` 與 `/market/qfii-holding-ranking`的測試。

//! 排行排序白名單的 deterministic tests。

use super::qfii_order_by;

/// §4.10 QFII 排序白名單：兩個固定分支皆為降冪加股票代號穩定排序；
/// 白名單以外的文字必須在接觸 SQL 前遭拒。
#[test]
fn qfii_sort_has_two_static_branches() {
    assert_eq!(
        qfii_order_by("percentage"),
        Ok("ORDER BY qfii_share_holding_percentage DESC, stock_symbol ASC")
    );
    assert_eq!(
        qfii_order_by("shares"),
        Ok("ORDER BY qfii_shares_held DESC, stock_symbol ASC")
    );
    for invalid in ["", "issued_share", "percentage; DROP TABLE stocks"] {
        assert!(qfii_order_by(invalid).is_err(), "{invalid:?} 應被拒絕");
    }
}
