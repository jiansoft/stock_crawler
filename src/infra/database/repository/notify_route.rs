//! 讀取通知路由（檢視表 `member_notify_route`，會員系統計畫 7.5）。

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::{
    domain::notify::{NotifyRoute, NotifyRouteRepository},
    infra::database,
};

/// 以 PostgreSQL 檢視表 `member_notify_route` 實作的路由來源。
#[derive(Debug, Default, Clone, Copy)]
pub struct PgNotifyRouteRepository;

#[async_trait]
impl NotifyRouteRepository for PgNotifyRouteRepository {
    async fn fetch_routes(&self, topic: &str) -> Result<Vec<NotifyRoute>> {
        let rows: Vec<(i64, i64, String, String)> = sqlx::query_as(
            r#"SELECT member_id, household_id, holdings_view, address
               FROM member_notify_route
               WHERE topic = $1 AND kind = 'telegram'
               ORDER BY member_id, channel_id"#,
        )
        .bind(topic)
        .fetch_all(database::get_connection())
        .await
        .with_context(|| format!("查詢通知路由 {topic} 失敗"))?;
        let mut routes = Vec::with_capacity(rows.len());
        for (member_id, household_id, holdings_view, address) in rows {
            // address 由 stock_go 寫入 Telegram chat_id；無法解析的列略過並記錄，不影響其他收件者。
            match address.parse::<i64>() {
                Ok(chat_id) => routes.push(NotifyRoute {
                    member_id,
                    household_id,
                    holdings_view,
                    chat_id,
                }),
                Err(_) => tracing::warn!("通知路由 {topic} 的會員 {member_id} 聊天室編號無法解析"),
            }
        }
        Ok(routes)
    }
}

#[cfg(test)]
mod tests;
