//! 通知送達採先佔位再發送；部分唯一索引保證不同執行也不會重送同一事件。

use anyhow::Result;
use async_trait::async_trait;
use sqlx::PgPool;

use crate::domain::notify::{
    DeliveryClaim, DeliveryOutcome, NotifyDeliveryRepository, NotifyRoute,
};

/// 以可注入的連線池操作送達紀錄，整合測試只使用隔離實例。
#[derive(Debug, Clone)]
pub struct PgNotifyDeliveryRepository {
    /// 目前服務或測試的連線池。
    pub pool: PgPool,
}

#[async_trait]
impl NotifyDeliveryRepository for PgNotifyDeliveryRepository {
    async fn reserve(
        &self,
        route: &NotifyRoute,
        topic: &str,
        key: &str,
    ) -> Result<Option<DeliveryClaim>> {
        let mut tx = self.pool.begin().await?;
        // 快取的系統路由可能剛被停用；在佔位時再次檢查 enabled，避免繼續送向失效管道。
        let id: Option<i64> = sqlx::query_scalar(
            "INSERT INTO member_notify_delivery (member_id, channel_id, topic, dedup_key, status)
             SELECT $1, id, $3, $4, 'pending' FROM member_notify_channel WHERE id = $2 AND enabled
             ON CONFLICT DO NOTHING RETURNING id",
        )
        .bind(route.member_id)
        .bind(route.channel_id)
        .bind(topic)
        .bind(key)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(id) = id else { return Ok(None) };
        let announce = if route.announce_new_topic {
            // 失敗的嘗試不算第一次送達；unknown 可能已送出，因此不能再加提示或重送。
            sqlx::query_scalar::<_, bool>(
                "SELECT NOT EXISTS (SELECT 1 FROM member_notify_delivery
                 WHERE member_id = $1 AND topic = $2 AND status IN ('sent', 'unknown'))",
            )
            .bind(route.member_id)
            .bind(topic)
            .fetch_one(&mut *tx)
            .await?
        } else {
            false
        };
        tx.commit().await?;
        Ok(Some(DeliveryClaim { id, announce }))
    }

    async fn finish(
        &self,
        claim: DeliveryClaim,
        route: &NotifyRoute,
        outcome: DeliveryOutcome,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let (status, code) = match outcome {
            DeliveryOutcome::Sent => ("sent", None),
            DeliveryOutcome::Failed { error_code, .. } => ("failed", error_code),
        };
        // 對帳已將它設為 unknown 時不再改回 failed，以免釋放「可能送過」的佔位。
        sqlx::query(
            "UPDATE member_notify_delivery SET status = $2, error_code = $3,
                     sent_at = CASE WHEN $2 = 'sent' THEN now() ELSE NULL END
                     WHERE id = $1 AND status = 'pending'",
        )
        .bind(claim.id)
        .bind(status)
        .bind(code)
        .execute(&mut *tx)
        .await?;
        match outcome {
            DeliveryOutcome::Sent => {
                sqlx::query("UPDATE member_notify_channel SET last_ok_at = now(), fail_count = 0 WHERE id = $1")
                    .bind(route.channel_id).execute(&mut *tx).await?;
            }
            DeliveryOutcome::Failed {
                disabled_reason, ..
            } => {
                sqlx::query(
                    "UPDATE member_notify_channel SET fail_count = fail_count + 1,
                             enabled = CASE WHEN $2::text IS NOT NULL THEN false ELSE enabled END,
                             disabled_reason = COALESCE($2, disabled_reason) WHERE id = $1",
                )
                .bind(route.channel_id)
                .bind(disabled_reason)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    async fn reconcile_pending(&self) -> Result<u64> {
        Ok(sqlx::query(
            "UPDATE member_notify_delivery SET status = 'unknown'
                        WHERE status = 'pending' AND created_at < now() - interval '30 minutes'",
        )
        .execute(&self.pool)
        .await?
        .rows_affected())
    }
}

#[cfg(test)]
mod tests;
