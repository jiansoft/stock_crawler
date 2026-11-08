//! 績效查詢型別的測試。

use super::*;

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
}

#[test]
fn sort_key_code_round_trips_and_rejects_unknown() {
    for key in CagrSortKey::ALL {
        assert_eq!(CagrSortKey::from_code(key.code()), Some(key));
    }
    assert_eq!(CagrSortKey::from_code("total_cagr_pct"), None);
    assert_eq!(CagrSortKey::default(), CagrSortKey::Cagr);
}

#[test]
fn short_periods_default_to_total_return() {
    // 3M/6M 年化後會被放大約四倍，預設排序必須退回區間總報酬。
    assert_eq!(
        CagrSortKey::default_for(CagrPeriod::M3),
        CagrSortKey::TotalReturn
    );
    assert_eq!(
        CagrSortKey::default_for(CagrPeriod::M6),
        CagrSortKey::TotalReturn
    );
    for period in [CagrPeriod::Y1, CagrPeriod::Y3, CagrPeriod::Y10] {
        assert_eq!(CagrSortKey::default_for(period), CagrSortKey::Cagr);
    }
}

#[test]
fn new_query_uses_total_metric_and_includes_incomplete() {
    let query = CagrRankingQuery::new(date(2026, 8, 7), CagrPeriod::Y1);

    assert_eq!(query.metric, CagrMetric::Total);
    assert_eq!(query.sort, CagrSortKey::Cagr);
    // 「查得到但算不出來」與「查不到」是兩件事，預設不可濾掉前者。
    assert!(query.include_incomplete);
    assert_eq!(query.limit, 50);
    assert_eq!(query.offset, 0);
    assert!(query.market_id.is_none());
    assert!(query.industry_id.is_none());
    assert!(query.keyword.is_none());
}

#[test]
fn new_query_picks_sort_key_from_period_length() {
    let short = CagrRankingQuery::new(date(2026, 8, 7), CagrPeriod::M3);
    assert_eq!(short.sort, CagrSortKey::TotalReturn);
}
