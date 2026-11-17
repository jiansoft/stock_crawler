use std::sync::Mutex;

use anyhow::anyhow;

use super::*;

#[derive(Default)]
struct RecordingSender {
    sent: Mutex<Vec<String>>,
}

#[async_trait]
impl HoldingsSender for RecordingSender {
    async fn send_legacy(&self, topic: Topic, message: &str) {
        self.sent
            .lock()
            .expect("測試鎖")
            .push(format!("legacy:{}:{message}", topic.code()));
    }

    async fn send_to_chat(&self, topic: Topic, chat_id: i64, message: &str) {
        self.sent
            .lock()
            .expect("測試鎖")
            .push(format!("{chat_id}:{}:{message}", topic.code()));
    }
}

struct FixedRoutes(Vec<NotifyRoute>);

#[async_trait]
impl NotifyRouteRepository for FixedRoutes {
    async fn fetch_routes(&self, _topic: &str) -> Result<Vec<NotifyRoute>> {
        Ok(self.0.clone())
    }
}

struct FailingRoutes;

#[async_trait]
impl NotifyRouteRepository for FailingRoutes {
    async fn fetch_routes(&self, _topic: &str) -> Result<Vec<NotifyRoute>> {
        Err(anyhow!("db down"))
    }
}

fn route(member_id: i64, household_id: i64, holdings_view: &str, chat_id: i64) -> NotifyRoute {
    NotifyRoute {
        member_id,
        household_id,
        holdings_view: holdings_view.to_string(),
        chat_id,
    }
}

/// 依範圍產生內容的替身：成員 3 沒有相關持股（回 None）、成員 4 產生失敗。
async fn build(view: HoldingView) -> Result<Option<String>> {
    match view.member_id() {
        None => Ok(Some(format!("家庭{}全體", view.household_id()))),
        Some(3) => Ok(None),
        Some(4) => Err(anyhow!("query failed")),
        Some(member_id) => Ok(Some(format!("成員{member_id}"))),
    }
}

#[test]
fn routed_setting_accepts_codes_and_all_but_only_for_holdings_and_personal() {
    assert!(is_routed(Topic::HoldingsPayable, "holdings.payable"));
    assert!(is_routed(
        Topic::HoldingsPayable,
        " market.weekly , holdings.payable "
    ));
    assert!(is_routed(Topic::HoldingsWeekly, "ALL"));
    assert!(is_routed(Topic::PersonalPriceTrace, "all"));
    assert!(!is_routed(Topic::HoldingsRevenue, "holdings.payable"));
    assert!(!is_routed(Topic::HoldingsPayable, ""));
    assert!(!is_routed(Topic::MarketWeekly, "all"));
    assert!(!is_routed(Topic::SystemCritical, "system.critical"));
}

/// 尚未切換：以預設家庭全體產生一次，送設定檔聊天室（Eddie 的內容不變）。
#[tokio::test]
async fn unrouted_topic_keeps_legacy_behaviour() {
    let sender = RecordingSender::default();
    let summary = dispatch_holdings(Topic::HoldingsPayable, "k", build, None, &sender).await;
    assert_eq!(summary, DispatchSummary { built: 1, sent: 1 });
    assert_eq!(
        *sender.sent.lock().expect("測試鎖"),
        ["legacy:holdings.payable:家庭1全體"]
    );

    let empty = RecordingSender::default();
    let summary = dispatch_holdings(
        Topic::HoldingsPayable,
        "k",
        |_| async { Ok(None) },
        None,
        &empty,
    )
    .await;
    assert_eq!(summary.sent, 0, "沒有內容時不發送");
}

/// 兩位收件者範圍不同時各自產生內容；同一家庭都選含家庭的人共用一份；沒有內容或失敗的範圍不發送（5-06 驗收）。
#[tokio::test]
async fn routed_topic_builds_once_per_view_and_sends_to_each_chat() {
    let routes = FixedRoutes(vec![
        route(1, 1, "household", 101),
        route(1, 1, "household", -102),
        route(5, 1, "household", 105),
        route(2, 1, "own", 201),
        route(3, 1, "own", 301),
        route(4, 1, "own", 401),
        route(7, 2, "household", 701),
        route(2, 1, "own", 201),
    ]);
    let sender = RecordingSender::default();
    let summary = dispatch_holdings(
        Topic::HoldingsPayable,
        "holdings.payable:2026-11-10",
        build,
        Some(&routes),
        &sender,
    )
    .await;
    assert_eq!(summary, DispatchSummary { built: 5, sent: 5 });
    assert_eq!(
        *sender.sent.lock().expect("測試鎖"),
        [
            "101:holdings.payable:家庭1全體",
            "-102:holdings.payable:家庭1全體",
            "105:holdings.payable:家庭1全體",
            "201:holdings.payable:成員2",
            "701:holdings.payable:家庭2全體",
        ]
    );
}

#[tokio::test]
async fn route_failure_sends_nothing() {
    let sender = RecordingSender::default();
    let summary = dispatch_holdings(
        Topic::HoldingsPayable,
        "k",
        build,
        Some(&FailingRoutes),
        &sender,
    )
    .await;
    assert_eq!(summary, DispatchSummary::default());
    assert!(sender.sent.lock().expect("測試鎖").is_empty());
}

#[tokio::test]
async fn legacy_build_error_is_logged_not_sent() {
    let sender = RecordingSender::default();
    let summary = dispatch_holdings(
        Topic::HoldingsPayable,
        "k",
        |_| async { Err::<Option<String>, _>(anyhow!("boom")) },
        None,
        &sender,
    )
    .await;
    assert_eq!(summary, DispatchSummary { built: 1, sent: 0 });
}
