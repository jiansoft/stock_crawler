use super::*;

fn route(holdings_view: &str) -> NotifyRoute {
    NotifyRoute {
        member_id: 2,
        household_id: 1,
        holdings_view: holdings_view.to_string(),
        chat_id: 900,
    }
}

#[test]
fn household_choice_sees_whole_household_and_others_only_self() {
    assert_eq!(route("household").view(), HoldingView::household(1));
    assert_eq!(route("own").view(), HoldingView::member(1, 2));
    assert_eq!(route("none").view(), HoldingView::member(1, 2));
}
