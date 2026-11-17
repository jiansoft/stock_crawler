//! 通知路由（會員系統計畫 7.5、7.6）：某個通知項目要送給誰、以什麼範圍產生內容。
//!
//! 路由由 stock_go 寫入的訂閱與管道組成（檢視表 `member_notify_route`），stock_rust 只讀。

use anyhow::Result;
use async_trait::async_trait;

use crate::domain::portfolio::view::HoldingView;

/// 一位收件者的一個聊天室。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyRoute {
    /// 收件會員。
    pub member_id: i64,
    /// 收件會員所屬家庭。
    pub household_id: i64,
    /// 通知管道編號，用於送達紀錄及失效停用。
    pub channel_id: i64,
    /// 此訂閱由背景對帳補上，首次送達時提示可在設定關閉。
    pub announce_new_topic: bool,
    /// 持股類內容範圍：`own`（只含本人）、`household`（含家庭成員）；市場與系統類為 `none`。
    pub holdings_view: String,
    /// Telegram 聊天室編號。
    pub chat_id: i64,
}

impl NotifyRoute {
    /// 收件者看到的持股範圍：選「含家庭成員」為整個家庭，其餘只有本人。
    pub fn view(&self) -> HoldingView {
        if self.holdings_view == "household" {
            HoldingView::household(self.household_id)
        } else {
            HoldingView::member(self.household_id, self.member_id)
        }
    }
}

/// 讀取通知路由。
#[async_trait]
pub trait NotifyRouteRepository: Send + Sync {
    /// 某個通知項目目前的收件者（已啟用、已改密、未刪除的會員，啟用中的管道，訂閱為開）。
    async fn fetch_routes(&self, topic: &str) -> Result<Vec<NotifyRoute>>;
}

/// 已成功佔位的一次送達；重複事件不會取得此識別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryClaim {
    /// 送達紀錄編號。
    pub id: i64,
    /// 此收件會員尚無成功送達紀錄，需要加上新項目提示。
    pub announce: bool,
}

/// 實際送達結果，只保存狀態與錯誤碼，不保存通知內容。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Sent,
    Failed {
        error_code: Option<i32>,
        disabled_reason: Option<&'static str>,
    },
}

/// 送達佔位與狀態的可靠儲存，正式實作為 PostgreSQL。
#[async_trait]
pub trait NotifyDeliveryRepository: Send + Sync {
    /// 原子插入 pending；已佔位／送達／unknown 或管道停用時回 None。
    async fn reserve(
        &self,
        route: &NotifyRoute,
        topic: &str,
        key: &str,
    ) -> Result<Option<DeliveryClaim>>;
    /// 更新本次結果與管道健康狀態。
    async fn finish(
        &self,
        claim: DeliveryClaim,
        route: &NotifyRoute,
        outcome: DeliveryOutcome,
    ) -> Result<()>;
    /// 將超過 30 分鐘的 pending 轉為 unknown；保留佔位，不重送。
    async fn reconcile_pending(&self) -> Result<u64>;
}

#[cfg(test)]
mod tests;
