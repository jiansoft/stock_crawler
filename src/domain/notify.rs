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

#[cfg(test)]
mod tests;
