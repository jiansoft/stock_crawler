//! 持股與個人類通知依收件者範圍分送（會員系統計畫 7.6、7.7 N4）。
//!
//! 呼叫點不再「組一則全體訊息再送」，而是提供一個依 [`HoldingView`] 組訊息的函式：
//!
//! - 主題已切換（列在 `NOTIFY_ROUTED_TOPICS`）：讀出該主題的路由，把收件者依範圍分組（同一家庭都選
//!   「含家庭成員」的人共用一份內容），每個不同的範圍呼叫一次 `build`，再送給該組的每個聊天室。
//!   範圍內沒有相關持股（`build` 回 `None`）就不發送。
//!   個人價格追蹤另經 [`notify_holdings_per_recipient`] 走相同流程，但不能合併不同會員的
//!   極端價格狀態；只合併同一收件會員的聊天室。
//! - 尚未切換：以「預設家庭、全體成員」呼叫一次 `build`，照舊送設定檔內的聊天室（N3 之前的行為）。
//!
//! `dedup_key` 是這一次事件的識別（例如 `holdings.payable:2026-11-10`）；送達紀錄的先佔位與重跑去重
//! 在 N5（5-08）接上，這一步只用於記錄。

use std::future::Future;

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    core::{
        alert,
        notify_topic::{Audience, Topic},
    },
    domain::{
        notify::{NotifyRoute, NotifyRouteRepository},
        portfolio::view::HoldingView,
    },
    infra::database::repository::notify_route::PgNotifyRouteRepository,
};

/// 已切換為依訂閱分送的主題：逗號分隔的項目代碼，`all` 代表全部持股與個人類項目；未設定時全部維持舊行為。
pub const ROUTED_TOPICS_ENV: &str = "NOTIFY_ROUTED_TOPICS";

/// 主題是否已切換為依訂閱分送（只有持股與個人類項目會切換；市場與系統類在 N5）。
pub fn is_routed(topic: Topic, setting: &str) -> bool {
    if !matches!(topic.audience(), Audience::Holdings | Audience::Personal) {
        return false;
    }
    setting
        .split(',')
        .map(str::trim)
        .any(|item| item.eq_ignore_ascii_case("all") || item == topic.code())
}

/// 送出通知的方式；正式環境為 [`alert`] 的 Telegram adapter，測試以替身記錄。
#[async_trait]
pub trait HoldingsSender: Send + Sync {
    /// 送到設定檔內的聊天室（尚未切換的主題）。
    async fn send_legacy(&self, topic: Topic, message: &str);

    /// 送到一位收件者的聊天室。
    async fn send_to_chat(&self, topic: Topic, chat_id: i64, message: &str);
}

/// 經 [`alert`] 送出（main 註冊的 Telegram adapter）。
#[derive(Debug, Default, Clone, Copy)]
pub struct AlertSender;

#[async_trait]
impl HoldingsSender for AlertSender {
    async fn send_legacy(&self, topic: Topic, message: &str) {
        alert::send_legacy(topic, message).await;
    }

    async fn send_to_chat(&self, topic: Topic, chat_id: i64, message: &str) {
        alert::send_to_chat(topic, chat_id, message).await;
    }
}

/// 一次分送的結果（記錄與測試用）。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DispatchSummary {
    /// 呼叫 `build` 的次數（不同範圍，個人類則是不同收件會員）。
    pub built: usize,
    /// 送出的訊息數（每個聊天室一則）。
    pub sent: usize,
}

/// 持股與個人類通知的入口：依收件者範圍各自產生內容並分送。
///
/// `build` 的錯誤只記錄，不影響其他範圍；通知失敗不讓排程失敗。
pub async fn notify_holdings<F, Fut>(topic: Topic, dedup_key: &str, build: F) -> DispatchSummary
where
    F: Fn(HoldingView) -> Fut,
    Fut: Future<Output = Result<Option<String>>>,
{
    let setting = std::env::var(ROUTED_TOPICS_ENV).unwrap_or_default();
    let routes = is_routed(topic, &setting)
        .then_some(&PgNotifyRouteRepository as &dyn NotifyRouteRepository);
    dispatch_holdings(topic, dedup_key, build, routes, &AlertSender).await
}

/// [`notify_holdings`] 的本體：`routes` 為 `None` 時走舊行為（設定檔聊天室、預設家庭全體）。
pub async fn dispatch_holdings<F, Fut>(
    topic: Topic,
    dedup_key: &str,
    build: F,
    routes: Option<&dyn NotifyRouteRepository>,
    sender: &dyn HoldingsSender,
) -> DispatchSummary
where
    F: Fn(HoldingView) -> Fut,
    Fut: Future<Output = Result<Option<String>>>,
{
    dispatch_for_recipient(
        topic,
        dedup_key,
        |view, _| build(view),
        routes,
        sender,
        false,
    )
    .await
}

/// 個人類通知沿用持股分送入口，但各收件會員獨立建立內容，以隔離極端價格去重狀態。
///
/// `None` 收件會員代表尚未切換的舊聊天室；同一會員的多個聊天室仍共用一次內容判斷。
pub async fn notify_holdings_per_recipient<F, Fut>(
    topic: Topic,
    event_key: &str,
    build: F,
) -> DispatchSummary
where
    F: Fn(HoldingView, Option<i64>) -> Fut,
    Fut: Future<Output = Result<Option<String>>>,
{
    let setting = std::env::var(ROUTED_TOPICS_ENV).unwrap_or_default();
    let routes = is_routed(topic, &setting)
        .then_some(&PgNotifyRouteRepository as &dyn NotifyRouteRepository);
    dispatch_for_recipient(topic, event_key, build, routes, &AlertSender, true).await
}

/// 共用分送流程；個人類按收件會員分組，其餘依可見範圍共用內容。
pub(crate) async fn dispatch_for_recipient<F, Fut>(
    topic: Topic,
    dedup_key: &str,
    build: F,
    routes: Option<&dyn NotifyRouteRepository>,
    sender: &dyn HoldingsSender,
    per_recipient: bool,
) -> DispatchSummary
where
    F: Fn(HoldingView, Option<i64>) -> Fut,
    Fut: Future<Output = Result<Option<String>>>,
{
    let mut summary = DispatchSummary::default();
    let Some(routes) = routes else {
        summary.built = 1;
        match build(HoldingView::default_household(), None).await {
            Ok(Some(message)) => {
                sender.send_legacy(topic, &message).await;
                summary.sent = 1;
            }
            Ok(None) => {}
            Err(error) => tracing::error!("{dedup_key}：產生 {} 通知失敗：{error:?}", topic.code()),
        }
        return summary;
    };

    let routes = match routes.fetch_routes(topic.code()).await {
        Ok(routes) => routes,
        Err(error) => {
            tracing::error!(
                "{dedup_key}：讀取 {} 的通知路由失敗：{error:?}",
                topic.code()
            );
            return summary;
        }
    };
    for (view, recipient, chats) in group_by_recipient(&routes, per_recipient) {
        summary.built += 1;
        let message = match build(view, recipient).await {
            Ok(Some(message)) => message,
            Ok(None) => continue,
            Err(error) => {
                // 不記錄範圍內的成員或內容；家庭編號足以追查。
                tracing::error!(
                    "{dedup_key}：產生 {} 通知失敗（家庭 {}）：{error:?}",
                    topic.code(),
                    view.household_id()
                );
                continue;
            }
        };
        for chat_id in chats {
            sender.send_to_chat(topic, chat_id, &message).await;
            summary.sent += 1;
        }
    }
    tracing::info!(
        "{dedup_key}：{} 依 {} 個範圍分送 {} 則",
        topic.code(),
        summary.built,
        summary.sent
    );
    summary
}

/// 收件者依範圍分組（保留路由順序）；個人類再按會員分組，每組的聊天室只送一次。
fn group_by_recipient(
    routes: &[NotifyRoute],
    per_recipient: bool,
) -> Vec<(HoldingView, Option<i64>, Vec<i64>)> {
    let mut groups: Vec<(HoldingView, Option<i64>, Vec<i64>)> = Vec::new();
    for route in routes {
        let view = route.view();
        let recipient = per_recipient.then_some(route.member_id);
        match groups
            .iter_mut()
            .find(|(existing, member, _)| *existing == view && *member == recipient)
        {
            Some((_, _, chats)) => {
                if !chats.contains(&route.chat_id) {
                    chats.push(route.chat_id);
                }
            }
            None => groups.push((view, recipient, vec![route.chat_id])),
        }
    }
    groups
}

#[cfg(test)]
mod tests;
