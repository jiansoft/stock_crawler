//! 持股可見範圍的測試。

use super::*;

#[test]
fn member_zero_or_negative_means_whole_household() {
    assert_eq!(HoldingView::member(2, 0), HoldingView::household(2));
    assert_eq!(HoldingView::member(2, -1).member_id(), None);
    assert_eq!(HoldingView::member(2, 5).member_id(), Some(5));
    assert_eq!(HoldingView::member(2, 5).household_id(), 2);
}

#[test]
fn default_household_is_household_one_with_total_member_id() {
    let view = HoldingView::default_household();
    assert_eq!(view.household_id(), DEFAULT_HOUSEHOLD_ID);
    assert_eq!(
        view.member_id_or_total(),
        0,
        "全體成員在回應裡仍是 member_id 0"
    );
    assert_eq!(HoldingView::member(1, 3).member_id_or_total(), 3);
}
