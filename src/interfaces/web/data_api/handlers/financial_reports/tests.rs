//! `/stocks/{symbol}/financial-reports` handler：單一股票的三大財務報表的測試。

use super::*;

/// 只接受 quarterly／annual；`all` 對三張分開的報表沒有意義，視為參數錯誤。
#[test]
fn period_type_maps_to_table_values() {
    assert_eq!(db_period_type("quarterly"), Some("single"));
    assert_eq!(db_period_type("annual"), Some("annual"));
    for invalid in ["all", "single", "Quarterly", ""] {
        assert_eq!(db_period_type(invalid), None, "{invalid:?} 應不合法");
    }
}

/// 期間標記沿用資料表的季別字串。
#[test]
fn period_label_joins_year_and_quarter() {
    assert_eq!(period_label(2026, "Q2"), "2026-Q2");
    assert_eq!(period_label(2025, "A"), "2025-A");
}
