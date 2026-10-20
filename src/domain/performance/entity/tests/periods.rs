//! CAGR 期間與指標代碼、年化與覆蓋率揭露規則的測試。

use super::*;

#[test]
fn period_code_round_trips_and_rejects_unknown() {
    for period in CagrPeriod::ALL {
        assert_eq!(
            CagrPeriod::from_code(period.code()),
            Some(period),
            "{} 應能由代碼還原",
            period.code()
        );
        assert!(!period.display_name().is_empty());
    }
    assert_eq!(CagrPeriod::from_code("Y8"), None);
    // 小寫不視為合法代碼，避免資料庫寫入大小寫混雜的值。
    assert_eq!(CagrPeriod::from_code("y1"), None);
}

#[test]
fn period_all_is_ordered_from_short_to_long() {
    let months: Vec<u32> = CagrPeriod::ALL.iter().map(|p| p.months()).collect();
    let mut sorted = months.clone();
    sorted.sort_unstable();
    assert_eq!(months, sorted, "ALL 必須依期間長度由短至長排列");
    assert_eq!(months.first(), Some(&3));
    assert_eq!(months.last(), Some(&120));
}

#[test]
fn short_periods_do_not_claim_meaningful_annualization() {
    assert!(!CagrPeriod::M3.annualization_is_meaningful());
    assert!(!CagrPeriod::M6.annualization_is_meaningful());
    assert!(CagrPeriod::Y1.annualization_is_meaningful());
    assert!(CagrPeriod::Y10.annualization_is_meaningful());
}

#[test]
fn long_periods_require_coverage_disclosure_and_drop_price_metric() {
    // Y5/Y10 涵蓋率僅約六成，且長期間配股普遍，兩項限制必須同時成立。
    for period in [CagrPeriod::Y5, CagrPeriod::Y7, CagrPeriod::Y10] {
        assert!(!period.is_default_exposed());
        assert!(period.requires_coverage_disclosure());
        assert!(!period.supports_price_metric());
    }
    for period in [CagrPeriod::M3, CagrPeriod::Y1, CagrPeriod::Y3] {
        assert!(period.is_default_exposed());
        assert!(!period.requires_coverage_disclosure());
        assert!(period.supports_price_metric());
    }
}

#[test]
fn metric_code_round_trips_and_rejects_unknown() {
    for metric in CagrMetric::ALL {
        assert_eq!(CagrMetric::from_code(metric.code()), Some(metric));
    }
    assert_eq!(CagrMetric::from_code("gross"), None);
}
