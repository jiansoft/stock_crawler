//! 追蹤條件的邊界判斷、警報去重（記憶體 TTL 與 Redis 雙層）與警報訊息。

use std::time::Duration;

use anyhow::Result;
use rust_decimal::Decimal;

use super::EvaluationSource;
use crate::app::event::trace::stats as trace_stats;
// 通知走 core::alert 抽象介面（port），MarkdownV2 跳脫用 core::util::text：
// app 層不 import interfaces::bot，實際送到 Telegram 由 main 註冊的 adapter 決定。
use crate::{
    core::alert,
    core::util::{map::Keyable, text},
    domain::portfolio::view::HoldingView,
    domain::trace::entity::PriceTrace,
    infra::cache::{SHARE, TTL, TtlCacheInner},
};

/// 同一則警報（同股票、同邊界方向）的去重時間窗（秒）。
///
/// 在此時間窗內，只有報價創新低（floor）或新高（ceiling）時才會再次發送警報；
/// 過了時間窗後極端值基準失效，相同價格才會再次提醒一次。預設為 1 小時。
const TRACE_ALERT_DEDUP_WINDOW_SECS: usize = 60 * 60;

/// 同一次報價的條件與來源；依每位收件者的可見範圍建立內容。
struct PriceAlert<'a> {
    targets: &'a [PriceTrace],
    current_price: Decimal,
    source_site: Option<&'a str>,
}

/// 極端價格去重的介面，測試可替換為不連 Redis 的記憶體實作。
#[async_trait::async_trait]
trait PriceDedup: Send + Sync {
    /// 僅在此收件者尚未收到、或價格更極端時允許通知。
    async fn should_notify(&self, key: &str, price: Decimal, lower_is_more_extreme: bool) -> bool;
}

/// 先以行程內原子比較，再以 Redis 保存跨重啟的極端值。
struct ExtremePriceDedup;

#[async_trait::async_trait]
impl PriceDedup for ExtremePriceDedup {
    async fn should_notify(&self, key: &str, price: Decimal, lower_is_more_extreme: bool) -> bool {
        if !TTL.trace_quote_notify_if_more_extreme(
            key.to_string(),
            price,
            lower_is_more_extreme,
            Duration::from_secs(TRACE_ALERT_DEDUP_WINDOW_SECS as u64),
        ) {
            return false;
        }
        match crate::infra::nosql::redis::CLIENT
            .set_if_more_extreme(
                key,
                price,
                TRACE_ALERT_DEDUP_WINDOW_SECS,
                lower_is_more_extreme,
            )
            .await
        {
            Ok(result) => result,
            Err(why) => {
                // Redis 故障仍保留上方的記憶體限流，避免盤中反覆洗版。
                tracing::error!("Failed to set Redis key {}: {:?}", key, why);
                trace_stats::record_redis_dedup_failure();
                true
            }
        }
    }
}

impl PriceAlert<'_> {
    /// 先按家庭與本人範圍篩選，再更新收件會員自己的極端值；家庭通知註明條件擁有者。
    async fn build(
        &self,
        view: HoldingView,
        recipient: Option<i64>,
        owners: &[(i64, String)],
        dedup: &dyn PriceDedup,
    ) -> Result<Option<String>> {
        let mut messages = Vec::new();
        for target in self.targets {
            let Some((_, name)) = owners.iter().find(|(id, _)| *id == target.member_id) else {
                continue;
            };
            if view.member_id().is_some_and(|id| id != target.member_id) {
                continue;
            }
            let Some(boundary) = triggered_boundary(target, self.current_price) else {
                continue;
            };
            // 收件者與條件擁有者都進鍵；同一家庭的兩位訂閱者互不壓掉通知。
            // 舊通道沿用原鍵，切換後另以收件者前綴隔離。
            let target_key = build_trace_notification_key(target, boundary);
            let key = match recipient {
                Some(id) => format!("TraceRecipient:{id}:{}:{target_key}", view.household_id()),
                None => target_key,
            };
            if !dedup
                .should_notify(&key, self.current_price, boundary == "floor")
                .await
            {
                continue;
            }
            let message = format_alert_message(target, self.current_price, self.source_site).await;
            if recipient.is_some() && view.member_id().is_none() {
                let label = if name.trim().is_empty() {
                    format!("Member {}", target.member_id)
                } else {
                    name.clone()
                };
                messages.push(format!(
                    "{} 的追蹤條件：{message}",
                    text::escape_markdown_v2(label)
                ));
            } else {
                // 舊通道保持原通知文字；本人範圍也不附加他人名稱。
                messages.push(message);
            }
        }
        Ok((!messages.is_empty()).then(|| messages.join("\n")))
    }
}

/// 同一代號的報價只取一次，統一將觸發條件按訂閱範圍分送；個人類不使用送達紀錄的 dedup_key。
pub(super) async fn alert_on_price_boundaries(
    targets: &[PriceTrace],
    current_price: Decimal,
    source: EvaluationSource,
    source_site: Option<&str>,
) -> Result<bool> {
    if !targets
        .iter()
        .any(|target| triggered_boundary(target, current_price).is_some())
    {
        return Ok(false);
    }
    let event = PriceAlert {
        targets,
        current_price,
        source_site,
    };
    let event_key = format!(
        "personal.price_trace:{}:{current_price}",
        targets[0].stock_symbol
    );
    let summary = crate::app::notify::notify_holdings_per_recipient(
        alert::Topic::PersonalPriceTrace,
        &event_key,
        |view, recipient| {
            let event = &event;
            async move {
                let owners =
                    crate::infra::database::repository::trace::fetch_visible_owners(view).await?;
                event
                    .build(view, recipient, &owners, &ExtremePriceDedup)
                    .await
            }
        },
    )
    .await;
    for _ in 0..summary.sent {
        trace_stats::record_notification_sent();
        if source == EvaluationSource::Reconciliation {
            trace_stats::record_reconciliation_alert_hit();
        }
    }
    Ok(summary.sent > 0)
}

/// 單筆條件的測試入口，正式流程以整個報價事件批次分送。
#[cfg(test)]
async fn alert_on_price_boundary(
    target: PriceTrace,
    current_price: Decimal,
    source: EvaluationSource,
    source_site: Option<&str>,
) -> Result<bool> {
    alert_on_price_boundaries(&[target], current_price, source, source_site).await
}

/// 價格落在區間外時回傳觸發方向；等於門檻與未設定的方向都不觸發。
fn triggered_boundary(target: &PriceTrace, price: Decimal) -> Option<&'static str> {
    if is_within_boundary(target, price) {
        None
    } else if target.floor > Decimal::ZERO && price < target.floor {
        Some("floor")
    } else if target.ceiling > Decimal::ZERO && price > target.ceiling {
        Some("ceiling")
    } else {
        None
    }
}

/// 格式化警報訊息內容。
async fn format_alert_message(
    target: &PriceTrace,
    current_price: Decimal,
    source_site: Option<&str>,
) -> String {
    let stock_name = SHARE
        .get_stock(&target.stock_symbol)
        .await
        .map_or_else(String::new, |stock| stock.name().to_string());

    let (boundary, limit) = if current_price < target.floor && target.floor > Decimal::ZERO {
        ("低於最低價", target.floor)
    } else {
        ("超過最高價", target.ceiling)
    };

    // 股名、價格等動態內容都要先做 MarkdownV2 跳脫（規則詳見
    // core::util::text::escape_markdown_v2 的 rustdoc），否則含保留字元
    // （如「-KY」的減號、小數點）的訊息會被 Telegram API 整則拒絕。
    let escaped_name = text::escape_markdown_v2(stock_name);
    let escaped_boundary = text::escape_markdown_v2(boundary.to_string());
    let escaped_limit = text::escape_markdown_v2(limit.to_string());
    let escaped_price = text::escape_markdown_v2(current_price.to_string());
    let escaped_source_site = text::escape_markdown_v2(
        source_site
            .filter(|site| !site.trim().is_empty())
            .unwrap_or("未知"),
    );
    let symbol = &target.stock_symbol;

    format!(
        "{escaped_name} {escaped_boundary}:{escaped_limit}，目前報價:{escaped_price}，採集站點:{escaped_source_site} [Yahoo 股市](https://tw\\.stock\\.yahoo\\.com/quote/{symbol})"
    )
}

/// 判斷當前價格是否在預定的 [floor, ceiling] 範圍內。
///
/// 如果設定值為 0，表示不限制該方向的邊界。
fn is_within_boundary(target: &PriceTrace, current_price: Decimal) -> bool {
    let floor = target.floor;
    let ceiling = target.ceiling;

    match (floor > Decimal::ZERO, ceiling > Decimal::ZERO) {
        (true, true) => current_price >= floor && current_price <= ceiling,
        (true, false) => current_price >= floor,
        (false, true) => current_price <= ceiling,
        _ => true, // 如果都沒設定，視為在範圍內
    }
}

/// 建立 trace 通知去重用的 key（記憶體 TTL 與 Redis 共用）。
///
/// Key 結構包含條件擁有者、股票、門檻與邊界方向（不含目前價格）：價格改以快取的「值」保存，
/// 作為已通知的極端值基準，讓同一方向的警報只在創新低/新高時才提醒。
///
/// 不含日期：去重的時效完全由時間窗 TTL（[`TRACE_ALERT_DEDUP_WINDOW_SECS`]）決定，
/// 額外加上日期對 1 小時的時間窗沒有實際作用，故省略以簡化 key。
fn build_trace_notification_key(target: &PriceTrace, boundary_type: &str) -> String {
    format!("{}:{}", target.key_with_prefix(), boundary_type)
}

#[cfg(test)]
mod tests;
