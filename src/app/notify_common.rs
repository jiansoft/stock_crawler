//! 市場通知即時讀路由；系統通知每五分鐘刷新快取，資料庫故障時沿用最後一份。

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::app::notify::{AlertSender, HoldingsSender, ROUTED_TOPICS_ENV, is_routed};
use crate::core::notify_topic::{Audience, Topic};
use crate::domain::notify::{NotifyRoute, NotifyRouteRepository};
use crate::infra::database::repository::notify_route::PgNotifyRouteRepository;

/// 系統快取的刷新間隔。
const REFRESH_INTERVAL: Duration = Duration::from_secs(300);
/// 系統路由全部查完才換快照，避免半份快取無法判斷開關。
type RouteSnapshot = HashMap<&'static str, Vec<NotifyRoute>>;

/// 最後一次嘗試時間與最後一份完整快照；成功查到零收件者也算有效快取。
#[derive(Default)]
struct CacheState {
    checked_at: Option<Instant>,
    snapshot: Option<RouteSnapshot>,
}

/// 可獨立建立的系統路由快取；測試注入失敗來源及虛擬時鐘。
#[derive(Default)]
pub struct SystemRouteCache {
    state: Mutex<CacheState>,
}

impl SystemRouteCache {
    /// 取得完整快照；刷新失敗保留舊資料，初次就失敗才回 None。
    async fn routes(
        &self,
        topic: Topic,
        repository: &dyn NotifyRouteRepository,
    ) -> Option<Vec<NotifyRoute>> {
        let mut state = self.state.lock().await;
        if state
            .checked_at
            .is_none_or(|last| last.elapsed() >= REFRESH_INTERVAL)
        {
            state.checked_at = Some(Instant::now());
            let queries = Topic::ALL
                .iter()
                .copied()
                .filter(|t| t.audience() == Audience::System)
                .map(|topic| async move {
                    Ok::<_, anyhow::Error>((
                        topic.code(),
                        repository.fetch_routes(topic.code()).await?,
                    ))
                });
            // 故障時不讓警報長時間等資料庫；所有讀取都失敗時也只等待一次上限。
            match tokio::time::timeout(
                Duration::from_secs(5),
                futures::future::try_join_all(queries),
            )
            .await
            {
                Ok(Ok(routes)) => state.snapshot = Some(routes.into_iter().collect()),
                Ok(Err(error)) => tracing::warn!("系統通知路由刷新失敗，沿用快取：{error:#}"),
                Err(_) => tracing::warn!("系統通知路由刷新逾時，沿用快取"),
            }
        }
        state
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.get(topic.code()).cloned().unwrap_or_default())
    }
}

/// 行程共用的系統快取，內容不包含任何通知本文。
static SYSTEM_ROUTES: std::sync::LazyLock<SystemRouteCache> =
    std::sync::LazyLock::new(SystemRouteCache::default);

/// 市場／系統入口，與持股共用送達紀錄與實際傳送端。
pub async fn notify_common(topic: Topic, message: &str) {
    let setting = std::env::var(ROUTED_TOPICS_ENV).unwrap_or_default();
    dispatch_common(
        topic,
        message,
        &setting,
        &PgNotifyRouteRepository,
        &SYSTEM_ROUTES,
        &AlertSender,
    )
    .await;
}

/// 注入路由與傳送端以驗證故障退回條件，不需要啟動真實機器人。
pub async fn dispatch_common(
    topic: Topic,
    message: &str,
    setting: &str,
    repository: &dyn NotifyRouteRepository,
    cache: &SystemRouteCache,
    sender: &dyn HoldingsSender,
) -> usize {
    if message.trim().is_empty() {
        return 0;
    }
    if !is_routed(topic, setting) {
        sender.send_legacy(topic, message).await;
        return 1;
    }
    let routes = if topic.audience() == Audience::System {
        match cache.routes(topic, repository).await {
            Some(routes) if !routes.is_empty() || topic != Topic::SystemCritical => routes,
            // 沒有任何可用快照，或關鍵警報確實無人訂閱，才退回設定檔。
            _ => {
                sender.send_legacy(topic, message).await;
                return 1;
            }
        }
    } else {
        match repository.fetch_routes(topic.code()).await {
            Ok(routes) => routes,
            Err(error) => {
                tracing::error!(topic = topic.code(), "市場通知路由讀取失敗：{error:#}");
                return 0;
            }
        }
    };
    let mut seen = HashSet::new();
    let sends = routes
        .iter()
        .filter(|route| seen.insert(route.chat_id))
        .map(|route| sender.send_to_route(topic, route, "", message));
    futures::future::join_all(sends)
        .await
        .into_iter()
        .filter(|sent| *sent)
        .count()
}

/// 讓系統路由即使沒有警報也定期刷新；runtime 結束時背景工作一起取消。
pub fn spawn_refresh() {
    tokio::spawn(async {
        let mut timer = tokio::time::interval(REFRESH_INTERVAL);
        loop {
            timer.tick().await;
            SYSTEM_ROUTES
                .routes(Topic::SystemCritical, &PgNotifyRouteRepository)
                .await;
        }
    });
}

#[cfg(test)]
mod tests;
