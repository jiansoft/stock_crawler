//! 系統快取、尊重關閉訂閱與故障退回的條件測試。

use super::*;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use std::sync::{
    Mutex as StdMutex,
    atomic::{AtomicBool, Ordering},
};

/// 可在載入後切換為故障，驗證快取保留最後成功的開關狀態。
struct Routes {
    fail: AtomicBool,
    rows: Vec<NotifyRoute>,
}

#[async_trait]
impl NotifyRouteRepository for Routes {
    async fn fetch_routes(&self, _: &str) -> Result<Vec<NotifyRoute>> {
        if self.fail.load(Ordering::SeqCst) {
            Err(anyhow!("db down"))
        } else {
            Ok(self.rows.clone())
        }
    }
}

/// 純記憶體傳送替身，legacy 以聊天室 0 表示。
#[derive(Default)]
struct Sender(StdMutex<Vec<i64>>);

#[async_trait]
impl HoldingsSender for Sender {
    async fn send_legacy(&self, _: Topic, _: &str) {
        self.0.lock().expect("測試鎖").push(0);
    }
    async fn send_to_chat(&self, _: Topic, chat: i64, _: &str) {
        self.0.lock().expect("測試鎖").push(chat);
    }
}

/// 快取中的有效路由。
fn row() -> NotifyRoute {
    NotifyRoute {
        member_id: 1,
        household_id: 1,
        channel_id: 1,
        announce_new_topic: false,
        holdings_view: "none".into(),
        chat_id: 101,
    }
}

/// 有快取後資料庫故障仍送原訂閱者，不退回其他聊天室。
#[tokio::test(start_paused = true)]
async fn cached_system_subscribers_survive_database_failure() {
    let routes = Routes {
        fail: AtomicBool::new(false),
        rows: vec![row()],
    };
    let cache = SystemRouteCache::default();
    let sender = Sender::default();
    dispatch_common(
        Topic::SystemDbHealth,
        "警報",
        "all",
        &routes,
        &cache,
        &sender,
    )
    .await;
    routes.fail.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(301)).await;
    dispatch_common(
        Topic::SystemDbHealth,
        "警報",
        "all",
        &routes,
        &cache,
        &sender,
    )
    .await;
    assert_eq!(*sender.0.lock().expect("測試鎖"), [101, 101]);
}

/// 有效空快取代表會員關閉通知；僅 critical 無人訂閱或啟動就讀不到資料庫才退回。
#[tokio::test(start_paused = true)]
async fn fallback_is_limited_to_no_snapshot_or_empty_critical() {
    let routes = Routes {
        fail: AtomicBool::new(false),
        rows: vec![],
    };
    let cache = SystemRouteCache::default();
    let sender = Sender::default();
    assert_eq!(
        dispatch_common(
            Topic::SystemHostHealth,
            "警報",
            "all",
            &routes,
            &cache,
            &sender
        )
        .await,
        0
    );
    routes.fail.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(301)).await;
    assert_eq!(
        dispatch_common(
            Topic::SystemHostHealth,
            "警報",
            "all",
            &routes,
            &cache,
            &sender
        )
        .await,
        0
    );
    dispatch_common(
        Topic::SystemCritical,
        "關鍵警報",
        "all",
        &routes,
        &cache,
        &sender,
    )
    .await;
    dispatch_common(
        Topic::SystemDbHealth,
        "啟動失敗",
        "all",
        &routes,
        &SystemRouteCache::default(),
        &sender,
    )
    .await;
    assert_eq!(*sender.0.lock().expect("測試鎖"), [0, 0]);
}

/// 市場路由故障不套用系統的退回條件；未切換的主題仍走舊聊天室。
#[tokio::test]
async fn market_failure_is_closed_and_unrouted_topics_keep_legacy_delivery() {
    let routes = Routes {
        fail: AtomicBool::new(true),
        rows: vec![],
    };
    let sender = Sender::default();
    let cache = SystemRouteCache::default();
    assert_eq!(
        dispatch_common(Topic::MarketWeekly, "市場", "all", &routes, &cache, &sender).await,
        0
    );
    dispatch_common(Topic::MarketWeekly, "市場", "", &routes, &cache, &sender).await;
    assert_eq!(*sender.0.lock().expect("測試鎖"), [0]);
}

/// 讀取很慢的路由來源：超過五秒才回應。
struct SlowRoutes;

#[async_trait]
impl NotifyRouteRepository for SlowRoutes {
    async fn fetch_routes(&self, _: &str) -> Result<Vec<NotifyRoute>> {
        tokio::time::sleep(Duration::from_secs(10)).await;
        Ok(vec![row()])
    }
}

/// 系統路由刷新逾時只等一次上限：沒有快照時退回設定檔聊天室。
#[tokio::test(start_paused = true)]
async fn slow_system_route_refresh_times_out_and_falls_back() {
    let _logs = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_test_writer()
            .finish(),
    );
    let sender = Sender::default();
    let sent = dispatch_common(
        Topic::SystemCritical,
        "關鍵警報",
        "all",
        &SlowRoutes,
        &SystemRouteCache::default(),
        &sender,
    )
    .await;
    assert_eq!(sent, 1);
    assert_eq!(*sender.0.lock().expect("測試鎖"), [0]);
}

/// 市場類讀到路由就逐一送出（同一聊天室只送一次）；空白本文不送。
#[tokio::test]
async fn market_routes_are_sent_once_per_chat_and_blank_is_skipped() {
    let routes = Routes {
        fail: AtomicBool::new(false),
        rows: vec![row(), row()],
    };
    let sender = Sender::default();
    let cache = SystemRouteCache::default();
    assert_eq!(
        dispatch_common(Topic::MarketWeekly, "  ", "all", &routes, &cache, &sender).await,
        0
    );
    assert_eq!(
        dispatch_common(Topic::MarketWeekly, "市場", "all", &routes, &cache, &sender).await,
        1
    );
    assert_eq!(*sender.0.lock().expect("測試鎖"), [101]);
}

/// 正式入口：測試環境沒有設定依訂閱分送，走設定檔聊天室；未註冊 Telegram adapter 時不送出、不出錯。
#[tokio::test]
async fn notify_common_entry_uses_legacy_chat_when_not_routed() {
    notify_common(Topic::MarketWeekly, "市場").await;
}
