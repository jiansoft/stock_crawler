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
    domain::trace::entity::PriceTrace,
    infra::cache::{SHARE, TTL, TtlCacheInner},
};

/// 同一則警報（同股票、同邊界方向）的去重時間窗（秒）。
///
/// 在此時間窗內，只有報價創新低（floor）或新高（ceiling）時才會再次發送警報；
/// 過了時間窗後極端值基準失效，相同價格才會再次提醒一次。預設為 1 小時。
const TRACE_ALERT_DEDUP_WINDOW_SECS: usize = 60 * 60;

/// 判斷股價是否觸發警報，並在必要時發送通知。
///
/// 去重規則（時間窗內，預設 1 小時，見 [`TRACE_ALERT_DEDUP_WINDOW_SECS`]）：
/// 對「同一股票、同一邊界方向」只在報價創新極端時才提醒——
/// - floor（低於最低價）：報價比時間窗內已通知的最低價更低時才提醒。
/// - ceiling（超過最高價）：報價比時間窗內已通知的最高價更高時才提醒。
///
/// 例如低標連續觸發 86.0 → 86.1 → 86.2 → 85.9：只在 86.0（首次）與 85.9（創新低）時各提醒一次。
/// 每次創新極端都會重置時間窗；過了時間窗後基準失效，相同價格才會再次提醒一次。
pub(super) async fn alert_on_price_boundary(
    target: PriceTrace,
    current_price: Decimal,
    source: EvaluationSource,
    source_site: Option<&str>,
) -> Result<bool> {
    // 判斷當前價格是否在預定範圍內（如果在範圍內則不需提醒）
    if is_within_boundary(&target, current_price) {
        return Ok(false);
    }

    // 判定是觸發高標還是低標
    let boundary_type = if current_price < target.floor && target.floor > Decimal::ZERO {
        "floor"
    } else if current_price > target.ceiling && target.ceiling > Decimal::ZERO {
        "ceiling"
    } else {
        // 理論上不會走到這裡，因為 above implies !is_within_boundary
        return Ok(false);
    };

    // floor（低於最低價）創新低才提醒；ceiling（超過最高價）創新高才提醒。
    let lower_is_more_extreme = boundary_type == "floor";
    let target_key = build_trace_notification_key(&target, boundary_type);

    // 第一層：記憶體 TTL 原子去重（本地、永不失敗），是防洗版的最後防線。
    // trace_quote_notify_if_more_extreme 以 and_compute_with 原子比較目前極端值，
    // 僅當報價比已記錄值更極端（floor 更低 / ceiling 更高）時才寫入並回報需通知，
    // 避免「同方向、未創極端」的報價持續洗版。
    let memory_fresh = TTL.trace_quote_notify_if_more_extreme(
        target_key.clone(),
        current_price,
        lower_is_more_extreme,
        Duration::from_secs(TRACE_ALERT_DEDUP_WINDOW_SECS as u64),
    );
    if !memory_fresh {
        return Ok(false);
    }

    // 第二層：Redis 去重（跨重啟／跨實例持久化）。只有報價比已記錄值更極端時才寫入，
    // 維持與記憶體層一致的「創新低/新高才通知」語意。
    // Redis 失敗時退回僅靠上方記憶體去重，避免洗版。
    let should_send = match crate::infra::nosql::redis::CLIENT
        .set_if_more_extreme(
            &target_key,
            current_price,
            TRACE_ALERT_DEDUP_WINDOW_SECS,
            lower_is_more_extreme,
        )
        .await
    {
        Ok(result) => result,
        Err(why) => {
            tracing::error!("Failed to set Redis key {}: {:?}", target_key, why);
            trace_stats::record_redis_dedup_failure();
            true
        }
    };

    if !should_send {
        return Ok(false);
    }

    // 格式化訊息並發送
    let to_bot_msg = format_alert_message(&target, current_price, source_site).await;

    // 透過 AlertSink port 發送通知（生產環境由 main 註冊 Telegram adapter）。
    alert::send_message(&to_bot_msg).await;
    trace_stats::record_notification_sent();
    if source == EvaluationSource::Reconciliation {
        trace_stats::record_reconciliation_alert_hit();
    }

    Ok(true)
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
/// Key 結構僅包含股票與邊界方向（不含價格）：價格改以快取的「值」保存，
/// 作為已通知的極端值基準，讓同一方向的警報只在創新低/新高時才提醒。
///
/// 不含日期：去重的時效完全由時間窗 TTL（[`TRACE_ALERT_DEDUP_WINDOW_SECS`]）決定，
/// 額外加上日期對 1 小時的時間窗沒有實際作用，故省略以簡化 key。
fn build_trace_notification_key(target: &PriceTrace, boundary_type: &str) -> String {
    format!("{}:{}", target.key_with_prefix(), boundary_type)
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    /// 驗證價格區間判斷邏輯可正確處理雙邊界、單邊界與未設定情況。
    #[test]
    fn test_is_within_boundary() {
        // 設定高低標 (500 ~ 600)
        let mut trace = PriceTrace {
            stock_symbol: "2330".to_string(),
            floor: dec!(500),
            ceiling: dec!(600),
        };

        // 邊界測試
        assert!(is_within_boundary(&trace, dec!(550)));
        assert!(is_within_boundary(&trace, dec!(500)));
        assert!(is_within_boundary(&trace, dec!(600)));
        assert!(!is_within_boundary(&trace, dec!(499.9)));
        assert!(!is_within_boundary(&trace, dec!(600.1)));

        // 僅設定低標 (>= 500)
        trace.ceiling = Decimal::ZERO;
        assert!(is_within_boundary(&trace, dec!(500)));
        assert!(is_within_boundary(&trace, dec!(1000)));
        assert!(!is_within_boundary(&trace, dec!(499.9)));

        // 僅設定高標 (<= 600)
        trace.floor = Decimal::ZERO;
        trace.ceiling = dec!(600);
        assert!(is_within_boundary(&trace, dec!(600)));
        assert!(is_within_boundary(&trace, dec!(0.1)));
        assert!(!is_within_boundary(&trace, dec!(600.1)));

        // 皆未設定
        trace.ceiling = Decimal::ZERO;
        assert!(is_within_boundary(&trace, dec!(123)));
    }

    #[test]
    fn test_build_trace_notification_key_includes_boundary() {
        let trace = PriceTrace {
            stock_symbol: "2330".to_string(),
            floor: dec!(25),
            ceiling: dec!(30),
        };

        // Key 僅含股票與邊界方向，不含價格（價格改以快取的值保存為極端值基準）。
        assert_eq!(
            build_trace_notification_key(&trace, "floor"),
            "Trace:2330-25-30:floor"
        );
        assert_eq!(
            build_trace_notification_key(&trace, "ceiling"),
            "Trace:2330-25-30:ceiling"
        );
    }

    #[test]
    fn test_build_trace_notification_key_distinguishes_boundary() {
        let trace = PriceTrace {
            stock_symbol: "2330".to_string(),
            floor: dec!(25),
            ceiling: dec!(30),
        };

        // 不同邊界方向應產生不同 key；同一方向不論價格皆為同一 key。
        assert_ne!(
            build_trace_notification_key(&trace, "floor"),
            build_trace_notification_key(&trace, "ceiling")
        );
    }

    #[tokio::test]
    async fn test_format_alert_message_includes_source_site() {
        let trace = PriceTrace {
            stock_symbol: "0050".to_string(),
            floor: dec!(0),
            ceiling: dec!(200),
        };

        let msg = format_alert_message(&trace, dec!(650), Some("Fugle")).await;

        assert!(msg.contains("超過最高價"));
        assert!(msg.contains("目前報價:650"));
        assert!(msg.contains("採集站點:Fugle"));
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_format_alert_message() {
        dotenvy::dotenv().ok();
        SHARE.load().await;

        let trace = PriceTrace {
            stock_symbol: "2330".to_string(),
            floor: dec!(500),
            ceiling: dec!(600),
        };

        // 觸發高標
        let msg = format_alert_message(&trace, dec!(650), Some("HiStock")).await;
        assert!(msg.contains("超過最高價"));
        assert!(msg.contains("目前報價:650"));
        assert!(msg.contains("採集站點:HiStock"));

        // 觸發低標
        let msg = msg_low(&trace, dec!(450), Some("Yahoo")).await;
        assert!(msg.contains("低於最低價"));
        assert!(msg.contains("目前報價:450"));
        assert!(msg.contains("採集站點:Yahoo"));
    }

    async fn msg_low(target: &PriceTrace, price: Decimal, source_site: Option<&str>) -> String {
        format_alert_message(target, price, source_site).await
    }

    #[tokio::test]
    #[ignore]
    async fn test_handle_price() {
        dotenvy::dotenv().ok();
        SHARE.load().await;

        let trace = PriceTrace {
            stock_symbol: "1303".to_string(),
            floor: dec!(70),
            ceiling: dec!(60),
        };

        let result = alert_on_price_boundary(
            trace,
            dec!(560),
            EvaluationSource::PriceEvent,
            Some("TestSite"),
        )
        .await;
        assert!(result.is_ok());
    }
}
